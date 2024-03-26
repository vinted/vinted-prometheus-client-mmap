# encoding: UTF-8

require 'prometheus/client/metric'

module Prometheus
  module Client
    # Counter is a metric that exposes merely a sum or tally of things.
    class Counter < Metric
      def type
        :counter
      end

      def increment(labels = {}, by = 1, exemplar_name = '', exemplar_value = '')
        raise ArgumentError, 'increment must be a non-negative number' if by < 0

        label_set = label_set_for(labels)
        synchronize { @values[label_set].increment(by, exemplar_name, exemplar_value) }
      end

      private

      def default(labels)
        value_object(type, @name, @name, labels)
      end
    end
  end
end
