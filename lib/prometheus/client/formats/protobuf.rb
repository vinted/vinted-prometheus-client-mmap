require 'prometheus/client/uses_value_type'
require 'prometheus/client/helper/json_parser'
require 'prometheus/client/helper/plain_file'
require 'prometheus/client/helper/metrics_processing'
require 'prometheus/client/helper/metrics_representation'

module Prometheus
  module Client
    module Formats
      # Protobuf format supports the same metric types as the text format plus
      # native histograms. It is only available with the Rust extension.
      module Protobuf
        MEDIA_TYPE = 'application/vnd.google.protobuf'.freeze
        PROTO = 'io.prometheus.client.MetricFamily'.freeze
        ENCODING = 'delimited'.freeze
        CONTENT_TYPE = "#{MEDIA_TYPE}; proto=#{PROTO}; encoding=#{ENCODING}".freeze

        class << self
          def marshal(registry)
            metrics = registry.metrics.map do |metric|
              samples = metric.values.flat_map do |label_set, value|
                representation(metric, label_set, value)
              end

              [metric.name, { type: metric.type, help: metric.docstring, samples: samples }]
            end

            Helper::MetricsRepresentation.to_metrics(metrics)
          end

          def marshal_multiprocess(path = Prometheus::Client.configuration.multiprocess_files_dir, use_rust: true)
            file_list = Dir.glob(File.join(path, '*.db')).sort
              .map {|f| Helper::PlainFile.new(f) }
              .map {|f| [f.filepath, f.multiprocess_mode.to_sym, f.type.to_sym, f.pid] }

            FastMmapedFileRs.to_protobuf(file_list.to_a)
          end

          def rust_impl_available?
            return @rust_available unless @rust_available.nil?

            check_for_rust
          end

          private

          def load_metrics(path)
            metrics = {}
            Dir.glob(File.join(path, '*.db')).sort.each do |f|
              Helper::PlainFile.new(f).to_metrics(metrics)
            end

            metrics
          end

          def representation(metric, label_set, value)
            labels = metric.base_labels.merge(label_set)

            if metric.type == :summary
              summary(metric.name, labels, value)
            elsif metric.type == :histogram
              histogram(metric.name, labels, value)
            else
              [[metric.name, labels, value.get]]
            end
          end

          def summary(name, set, value)
            rv = value.get.map do |q, v|
              [name, set.merge(quantile: q), v]
            end

            rv << ["#{name}_sum", set, value.get.sum]
            rv << ["#{name}_count", set, value.get.total]
            rv
          end

          def histogram(name, set, value)
            # |metric_name, labels, value|
            rv = value.get.map do |q, v|
              [name, set.merge(le: q), v]
            end

            rv << ["#{name}_sum", set, value.get.sum]
            rv << ["#{name}_count", set, value.get.total]
            rv
          end
        end
      end
    end
  end
end
