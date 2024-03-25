require 'prometheus/client/helper/entry_parser'

module Prometheus
  module Client
    module Helper
      # Parses DB files without using mmap
      class PlainFile
        include EntryParser
        attr_reader :filepath

        def source
          @data ||= File.read(filepath, mode: 'rb')
        end

        def initialize(filepath)
          @filepath = filepath
        end

        def slice(*args)
          source.slice(*args)
        end

        def size
          source.length
        end
      end
    end
  end
end
