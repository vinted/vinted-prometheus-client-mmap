require 'prometheus/client/registry'
require 'prometheus/client/mmaped_value'
require 'prometheus/client/page_size'
require 'logger'
require 'tmpdir'

module Prometheus
  module Client
    class Configuration
      attr_accessor :value_class, :multiprocess_files_dir, :initial_mmap_file_size, :logger, :pid_provider

      def initialize
        @value_class = ::Prometheus::Client::MmapedValue
        @initial_mmap_file_size = ::Prometheus::Client::PageSize.page_size(fallback_page_size: 4096)
        @logger = Logger.new($stdout)
        @pid_provider = Process.method(:pid)
        @multiprocess_files_dir = ENV.fetch('prometheus_multiproc_dir') do
          Dir.mktmpdir("prometheus-mmap")
        end
      end
    end
  end
end
