require 'open3'

module Prometheus
  module Client
    module PageSize
      def self.page_size(fallback_page_size: 4096)
        stdout, status = Open3.capture2('getconf PAGESIZE')
        return fallback_page_size if status.nil? || !status.success?

        page_size = stdout.chomp.to_i
        return fallback_page_size if page_size <= 0

        page_size
      end
    end
  end
end
