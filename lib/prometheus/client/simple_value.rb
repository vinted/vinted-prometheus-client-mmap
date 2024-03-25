require 'json'

module Prometheus
  module Client
    class SimpleValue
      def initialize(_type, _metric_name, _name, _labels, *_args)
        @value = 0.0
      end

      def set(value)
        @value = value
      end

      def increment(by = 1)
        @value += by
      end

      def decrement(by = 1)
        @value -= by
      end

      def get
        @value
      end

      def self.multiprocess
        false
      end
    end
  end
end
