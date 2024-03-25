require 'prometheus/client/helper/mmaped_file'
require 'prometheus/client/helper/plain_file'
require 'prometheus/client'

module Prometheus
  module Client
    class ParsingError < StandardError
    end

    # A dict of doubles, backed by an mmapped file.
    #
    # The file starts with a 4 byte int, indicating how much of it is used.
    # Then 4 bytes of padding.
    # There's then a number of entries, consisting of a 4 byte int which is the
    # size of the next field, a utf-8 encoded string key, padding to an 8 byte
    # alignment, and then a 8 byte float which is the value.
    class MmapedDict
      attr_reader :m, :used, :positions

      def self.read_all_values(f)
        Helper::PlainFile.new(f).entries.map do |data, encoded_len, value_offset, _|
          encoded, value = data.unpack(format('@4A%d@%dd', encoded_len, value_offset))
          [encoded, value]
        end
      end

      def initialize(m)
        @mutex = Mutex.new

        @m = m
        # @m.mlock # TODO: Ensure memory is locked to RAM

        @positions = {}
        read_all_positions.each do |key, pos|
          @positions[key] = pos
        end
      rescue StandardError => e
        raise ParsingError, "exception #{e} while processing metrics file #{path}"
      end

      def read_value(key)
        @m.fetch_entry(@positions, key, 0.0)
      end

      def write_value(key, value)
        @m.upsert_entry(@positions, key, value)
      end

      def write_exemplar(key, value, exemplar_id, exemplar_val)
        @m.upsert_exemplar({}, key, value, exemplar_id, exemplar_val)
      end

      def path
        @m.filepath if @m
      end

      def close
        @m.sync
        @m.close
      rescue TypeError => e
        Prometheus::Client.logger.warn("munmap raised error #{e}")
      end

      def inspect
        "#<#{self.class}:0x#{(object_id << 1).to_s(16)}>"
      end

      private

      def init_value(key)
        @m.add_entry(@positions, key, 0.0)
      end

      # Yield (key, pos). No locking is performed.
      def read_all_positions
        @m.entries.map do |data, encoded_len, _, absolute_pos|
          encoded, = data.unpack(format('@4A%d', encoded_len))
          [encoded, absolute_pos]
        end
      end
    end
  end
end
