module Prometheus
  module Client
    module Helper
      module MetricsProcessing
        def self.merge_metrics(metrics)
          metrics.each_value do |metric|
            metric[:samples] = merge_samples(metric[:samples], metric[:type], metric[:multiprocess_mode]).map do |(name, labels), value|
              [name, labels.to_h, value]
            end
          end
        end

        def self.merge_samples(raw_samples, metric_type, multiprocess_mode)
          samples = {}
          raw_samples.each do |name, labels, value|
            without_pid = labels.reject { |l| l[0] == 'pid' }

            case metric_type
              when :gauge
                case multiprocess_mode
                  when 'min'
                    s = samples.fetch([name, without_pid], value)
                    samples[[name, without_pid]] = [s, value].min
                  when 'max'
                    s = samples.fetch([name, without_pid], value)
                    samples[[name, without_pid]] = [s, value].max
                  when 'livesum'
                    s = samples.fetch([name, without_pid], 0.0)
                    samples[[name, without_pid]] = s + value
                  else # all/liveall
                    samples[[name, labels]] = value
                end
              else
                # Counter, Histogram and Summary.
                s = samples.fetch([name, without_pid], 0.0)
                samples[[name, without_pid]] = s + value
            end
          end

          samples
        end
      end
    end
  end
end
