require 'thread'
require 'prometheus/client/label_set_validator'
require 'prometheus/client/uses_value_type'

module Prometheus
  module Client
    class Metric
      include UsesValueType
      attr_reader :name, :docstring, :base_labels

      def initialize(name, docstring, base_labels = {})
        @mutex = Mutex.new
        @validator = case type
                       when :summary
                         LabelSetValidator.new(['quantile'])
                       when :histogram
                         LabelSetValidator.new(['le'])
                       else
                         LabelSetValidator.new
                     end
        @values = Hash.new { |hash, key| hash[key] = default(key) }

        validate_name(name)
        validate_docstring(docstring)
        @validator.valid?(base_labels)

        @name = name
        @docstring = docstring
        @base_labels = base_labels
      end

      # Returns the value for the given label set
      def get(labels = {})
        label_set = label_set_for(labels)
        @validator.valid?(label_set)

        @values[label_set].get
      end

      # Returns all label sets with their values
      def values
        synchronize do
          @values.each_with_object({}) do |(labels, value), memo|
            memo[labels] = value
          end
        end
      end

      private

      def touch_default_value
        @values[label_set_for({})]
      end

      def default(labels)
        value_object(type, @name, @name, labels)
      end

      def validate_name(name)
        return true if name.is_a?(Symbol)

        raise ArgumentError, 'given name must be a symbol'
      end

      def validate_docstring(docstring)
        return true if docstring.respond_to?(:empty?) && !docstring.empty?

        raise ArgumentError, 'docstring must be given'
      end

      def label_set_for(labels)
        @validator.validate(@base_labels.merge(labels))
      end

      def synchronize(&block)
        @mutex.synchronize(&block)
      end
    end
  end
end
