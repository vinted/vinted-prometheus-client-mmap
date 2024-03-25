module Prometheus
  module Client
    module Helper
      class FileLocker
        class << self
          LOCK_FILE_MUTEX = Mutex.new

          def lock_to_process(filepath)
            LOCK_FILE_MUTEX.synchronize do
              @file_locks ||= {}
              return false if @file_locks[filepath]

              file = File.open(filepath, 'ab')
              if file.flock(File::LOCK_NB | File::LOCK_EX)
                @file_locks[filepath] = file
                return true
              else
                return false
              end
            end
          end

          def unlock(filepath)
            LOCK_FILE_MUTEX.synchronize do
              @file_locks ||= {}
              return false unless @file_locks[filepath]

              file = @file_locks[filepath]
              file.flock(File::LOCK_UN)
              file.close
              @file_locks.delete(filepath)
            end
          end

          def unlock_all
            LOCK_FILE_MUTEX.synchronize do
              @file_locks ||= {}
              @file_locks.values.each do |file|
                file.flock(File::LOCK_UN)
                file.close
              end

              @file_locks = {}
            end
          end
        end
      end
    end
  end
end
