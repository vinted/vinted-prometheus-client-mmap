module Prometheus
  module Client
    module Helper
      module MetricsRepresentation
        METRIC_LINE = '%s%s %s'.freeze
        TYPE_LINE = '# TYPE %s %s'.freeze
        HELP_LINE = '# HELP %s %s'.freeze

        LABEL = '%s="%s"'.freeze
        SEPARATOR = ','.freeze
        DELIMITER = "\n".freeze

        REGEX = { doc: /[\n\\]/, label: /[\n\\"]/ }.freeze
        REPLACE = { "\n" => '\n', '\\' => '\\\\', '"' => '\"' }.freeze

        def self.to_text(metrics)
          lines = []

          metrics.each do |name, metric|
            lines << format(HELP_LINE, name, escape(metric[:help]))
            lines << format(TYPE_LINE, name, metric[:type])
            metric[:samples].each do |metric_name, labels, value|
              lines << metric(metric_name, format_labels(labels), value)
            end
          end

          # there must be a trailing delimiter
          (lines << nil).join(DELIMITER)
        end

        def self.metric(name, labels, value)
          format(METRIC_LINE, name, labels, value)
        end

        def self.format_labels(set)
          return if set.empty?

          strings = set.each_with_object([]) do |(key, value), memo|
            memo << format(LABEL, key, escape(value, :label))
          end

          "{#{strings.join(SEPARATOR)}}"
        end

        def self.escape(string, format = :doc)
          string.to_s.gsub(REGEX[format], REPLACE)
        end
      end
    end
  end
end
