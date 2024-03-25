module Prometheus
  module Client
    module Support
      module Puma
        extend self

        def worker_pid_provider
          wid = worker_id
          if wid = worker_id
            wid
          else
            "process_id_#{Process.pid}"
          end
        end

        private

        def object_based_worker_id
          return unless defined?(::Puma::Cluster::Worker)

          workers = ObjectSpace.each_object(::Puma::Cluster::Worker)
          return if workers.nil?

          workers_first = workers.first
          workers_first.index unless workers_first.nil?
        end

        def program_name
          $PROGRAM_NAME
        end

        def worker_id
          if matchdata = program_name.match(/puma.*cluster worker ([0-9]+):/)
            "puma_#{matchdata[1]}"
          elsif object_worker_id = object_based_worker_id
            "puma_#{object_worker_id}"
          elsif program_name.include?('puma')
            'puma_master'
          end
        end
      end
    end
  end
end
