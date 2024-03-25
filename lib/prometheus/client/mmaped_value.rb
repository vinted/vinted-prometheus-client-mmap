require 'prometheus/client'
require 'prometheus/client/mmaped_dict'
require 'json'

module Prometheus
  module Client
    # A float protected by a mutex backed by a per-process mmaped file.
    class MmapedValue
      VALUE_LOCK = Mutex.new

      @@files = {}
      @@pid = -1

      def initialize(type, metric_name, name, labels, multiprocess_mode = '')
        @file_prefix = type.to_s
        @metric_name = metric_name
        @name = name
        @labels = labels
        if type == :gauge
          @file_prefix += '_' + multiprocess_mode.to_s
        end

        @pid = -1

        @mutex = Mutex.new
        initialize_file
      end

      def increment(amount = 1, exemplar_name = '', exemplar_value = '')
        @mutex.synchronize do
          initialize_file if pid_changed?

          @value += amount
          # TODO(GiedriusS): write exemplars too.
          if @file_prefix != 'gauge'
            puts "#{@name} exemplar name = #{exemplar_name}, exemplar_value = #{exemplar_value}"
          end
          write_value(@key, @value)
          @value
        end
      end

      def decrement(amount = 1)
        increment(-amount)
      end

      def set(value)
        @mutex.synchronize do
          initialize_file if pid_changed?

          @value = value
          write_value(@key, @value)
          @value
        end
      end

      def get
        @mutex.synchronize do
          initialize_file if pid_changed?
          return @value
        end
      end

      def pid_changed?
        @pid != Process.pid
      end

      # method needs to be run in VALUE_LOCK mutex
      def unsafe_reinitialize_file(check_pid = true)
        unsafe_initialize_file if !check_pid || pid_changed?
      end

      def self.reset_and_reinitialize
        VALUE_LOCK.synchronize do
          @@pid = Process.pid
          @@files = {}

          ObjectSpace.each_object(MmapedValue).each do |v|
            v.unsafe_reinitialize_file(false)
          end
        end
      end

      def self.reset_on_pid_change
        if pid_changed?
          @@pid = Process.pid
          @@files = {}
        end
      end

      def self.reinitialize_on_pid_change
        VALUE_LOCK.synchronize do
          reset_on_pid_change

          ObjectSpace.each_object(MmapedValue, &:unsafe_reinitialize_file)
        end
      end

      def self.pid_changed?
        @@pid != Process.pid
      end

      def self.multiprocess
        true
      end

      private

      def initialize_file
        VALUE_LOCK.synchronize do
          unsafe_initialize_file
        end
      end

      def unsafe_initialize_file
        self.class.reset_on_pid_change

        @pid = Process.pid
        unless @@files.has_key?(@file_prefix)
          unless @file.nil?
            @file.close
          end
          unless @exemplar_file.nil?
            @exemplar_file.close
          end
          mmaped_file = Helper::MmapedFile.open_exclusive_file(@file_prefix)
          exemplar_file = Helper::MmapedFile.open_exclusive_file('exemplar')

          @@files[@file_prefix] = MmapedDict.new(mmaped_file)
          @@files['exemplar'] = MmapedDict.new(exemplar_file)
        end

        @file = @@files[@file_prefix]
        @exemplar_file = @@files['exemplar']
        @key = rebuild_key

        @value = read_value(@key)
      end


      def rebuild_key
        keys = @labels.keys.sort
        values = @labels.values_at(*keys)

        [@metric_name, @name, keys, values].to_json
      end

      def write_value(key, val)
        @file.write_value(key, val)
        puts "#{key} #{val}"
        @exemplar_file.m.upsert_exemplar({}, key, val, "foo", "bar")
      rescue StandardError => e
        Prometheus::Client.logger.warn("writing value to #{@file.path} failed with #{e}")
        Prometheus::Client.logger.debug(e.backtrace.join("\n"))
      end

      def read_value(key)
        @file.read_value(key)
      rescue StandardError => e
        Prometheus::Client.logger.warn("reading value from #{@file.path} failed with #{e}")
        Prometheus::Client.logger.debug(e.backtrace.join("\n"))
        0
      end
    end
  end
end
