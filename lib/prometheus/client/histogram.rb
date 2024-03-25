require 'prometheus/client/metric'
require 'prometheus/client/uses_value_type'

module Prometheus
  module Client
    # A histogram samples observations (usually things like request durations
    # or response sizes) and counts them in configurable buckets. It also
    # provides a sum of all observed values.
    class Histogram < Metric
      # Value represents the state of a Histogram at a given point.
      class Value < Hash
        include UsesValueType
        attr_accessor :sum, :total, :total_inf

        def initialize(type, name, labels, buckets)
          @sum = value_object(type, name, "#{name}_sum", labels)
          @total = value_object(type, name, "#{name}_count", labels)
          @total_inf = value_object(type, name, "#{name}_bucket", labels.merge(le: "+Inf"))

          buckets.each do |bucket|
            self[bucket] = value_object(type, name, "#{name}_bucket", labels.merge(le: bucket.to_s))
          end
        end

        def observe(value)
          @sum.increment(value)
          @total.increment()
          @total_inf.increment()

          each_key do |bucket|
            self[bucket].increment() if value <= bucket
          end
        end

        def get()
          hash = {}
          each_key do |bucket|
            hash[bucket] = self[bucket].get()
          end
          hash
        end
      end

      # DEFAULT_BUCKETS are the default Histogram buckets. The default buckets
      # are tailored to broadly measure the response time (in seconds) of a
      # network service. (From DefBuckets client_golang)
      DEFAULT_BUCKETS = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1,
                         2.5, 5, 10].freeze

      # Offer a way to manually specify buckets
      def initialize(name, docstring, base_labels = {},
                     buckets = DEFAULT_BUCKETS)
        raise ArgumentError, 'Unsorted buckets, typo?' unless sorted? buckets

        @buckets = buckets
        super(name, docstring, base_labels)
      end

      def type
        :histogram
      end

      def observe(labels, value)
        label_set = label_set_for(labels)
        synchronize { @values[label_set].observe(value) }
      end

      private

      def default(labels)
        # TODO: default function needs to know key of hash info (label names and values)
        Value.new(type, @name, labels, @buckets)
      end

      def sorted?(bucket)
        bucket.each_cons(2).all? { |i, j| i <= j }
      end
    end
  end
end
