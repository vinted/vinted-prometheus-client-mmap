require 'prometheus/client/helper/entry_parser'
require 'prometheus/client/helper/file_locker'

# load precompiled extension if available
begin
  ruby_version = /(\d+\.\d+)/.match(RUBY_VERSION)
  require_relative "../../../#{ruby_version}/fast_mmaped_file_rs"
rescue LoadError
  require 'fast_mmaped_file_rs'
end

module Prometheus
  module Client
    module Helper
      class MmapedFile < FastMmapedFileRs
        include EntryParser

        attr_reader :filepath, :size

        def initialize(filepath)
          @filepath = filepath

          File.open(filepath, 'a+b') do |file|
            file.truncate(initial_mmap_file_size) if file.size < MINIMUM_SIZE
            @size = file.size
          end

          super(filepath)
        end

        def close
          munmap
          FileLocker.unlock(filepath)
        end

        private

        def initial_mmap_file_size
          Prometheus::Client.configuration.initial_mmap_file_size
        end

        public

        class << self
          def open(filepath)
            MmapedFile.new(filepath)
          end

          def ensure_exclusive_file(file_prefix = 'mmaped_file')
            (0..Float::INFINITY).lazy
              .map { |f_num| "#{file_prefix}_#{Prometheus::Client.pid}-#{f_num}.db" }
              .map { |filename| File.join(Prometheus::Client.configuration.multiprocess_files_dir, filename) }
              .find { |path| Helper::FileLocker.lock_to_process(path) }
          end

          def open_exclusive_file(file_prefix = 'mmaped_file')
            filename = Helper::MmapedFile.ensure_exclusive_file(file_prefix)
            open(filename)
          end
        end
      end
    end
  end
end
