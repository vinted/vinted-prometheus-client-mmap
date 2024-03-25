module Prometheus
  module Client
    module Support
      module Unicorn
        def self.worker_pid_provider
          wid = worker_id
          if wid.nil?
            "process_id_#{Process.pid}"
          else
            "worker_id_#{wid}"
          end
        end

        def self.worker_id
          match = $0.match(/worker\[([^\]]+)\]/)
          if match
            match[1]
          else
            object_based_worker_id
          end
        end

        def self.object_based_worker_id
          return unless defined?(::Unicorn::Worker)

          workers = ObjectSpace.each_object(::Unicorn::Worker)
          return if workers.nil?

          workers_first = workers.first
          workers_first.nr unless workers_first.nil?
        end
      end
    end
  end
end
