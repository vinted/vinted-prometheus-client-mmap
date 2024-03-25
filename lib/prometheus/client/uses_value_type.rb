require 'prometheus/client/simple_value'

module Prometheus
  module Client
    # Module providing convenience methods for creating value_object
    module UsesValueType
      def value_class
        Prometheus::Client.configuration.value_class
      end

      def value_object(type, metric_name, name, labels, *args)
        value_class.new(type, metric_name, name, labels, *args)
      rescue StandardError => e
        Prometheus::Client.logger.info("error #{e} while creating instance of #{value_class} defaulting to SimpleValue")
        Prometheus::Client.logger.debug("error #{e} backtrace #{e.backtrace.join("\n")}")
        Prometheus::Client::SimpleValue.new(type, metric_name, name, labels)
      end
    end
  end
end
