# encoding: UTF-8

module Prometheus
  module Client
    # LabelSetValidator ensures that all used label sets comply with the
    # Prometheus specification.
    class LabelSetValidator
      RESERVED_LABELS = [].freeze

      class LabelSetError < StandardError; end
      class InvalidLabelSetError < LabelSetError; end
      class InvalidLabelError < LabelSetError; end
      class ReservedLabelError < LabelSetError; end

      def initialize(reserved_labels = [])
        @reserved_labels = (reserved_labels + RESERVED_LABELS).freeze
        @validated = {}
      end

      def valid?(labels)
        unless labels.is_a?(Hash)
          raise InvalidLabelSetError, "#{labels} is not a valid label set"
        end

        labels.all? do |key, value|
          validate_symbol(key)
          validate_name(key)
          validate_reserved_key(key)
          validate_value(key, value)
        end
      end

      def validate(labels)
        return labels if @validated.key?(labels.hash)

        valid?(labels)

        unless @validated.empty? || match?(labels, @validated.first.last)
          raise InvalidLabelSetError, "labels must have the same signature: (#{label_diff(labels, @validated.first.last)})"
        end

        @validated[labels.hash] = labels
      end

      private

      def label_diff(a, b)
        "expected keys: #{b.keys.sort}, got: #{a.keys.sort}"
      end

      def match?(a, b)
        a.keys.sort == b.keys.sort
      end

      def validate_symbol(key)
        return true if key.is_a?(Symbol)

        raise InvalidLabelError, "label #{key} is not a symbol"
      end

      def validate_name(key)
        return true unless key.to_s.start_with?('__')

        raise ReservedLabelError, "label #{key} must not start with __"
      end

      def validate_reserved_key(key)
        return true unless @reserved_labels.include?(key)

        raise ReservedLabelError, "#{key} is reserved"
      end

      def validate_value(key, value)
        return true if value.is_a?(String) ||
                       value.is_a?(Numeric) ||
                       value.is_a?(Symbol) ||
                       value.is_a?(FalseClass) ||
                       value.is_a?(TrueClass) ||
                       value.nil?

        raise InvalidLabelError, "#{key} does not contain a valid value (type #{value.class})"
      end
    end
  end
end
