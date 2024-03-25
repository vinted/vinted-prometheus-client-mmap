require 'prometheus/client/mmaped_dict'
require 'prometheus/client/page_size'
require 'tempfile'

describe Prometheus::Client::MmapedValue, :temp_metrics_dir do
  before do
    allow(Prometheus::Client.configuration).to receive(:multiprocess_files_dir).and_return(temp_metrics_dir)
  end

  describe '.reset_and_reinitialize' do
    let(:counter) { described_class.new(:counter, :counter, 'counter', {}) }

    before do
      # reset all current metrics
      Prometheus::Client::MmapedValue.class_variable_set(:@@files, {})

      counter.increment(1)
    end

    it 'calls reinitialize on the counter' do
      expect(counter).to receive(:unsafe_reinitialize_file).with(false).and_call_original

      described_class.reset_and_reinitialize
    end

    context 'when metrics folder changes' do
      around do |example|
        Dir.mktmpdir('temp_metrics_dir') do |path|
          @tmp_path = path

          example.run
        end
      end

      before do
        allow(Prometheus::Client.configuration).to receive(:multiprocess_files_dir).and_return(@tmp_path)
      end

      it 'resets the counter to zero' do
        expect(counter).to receive(:unsafe_reinitialize_file).with(false).and_call_original

        expect { described_class.reset_and_reinitialize }.to(change { counter.get }.from(1).to(0))
      end
    end
  end

  describe '#initialize' do
    let(:pid) { 1234 }
    before do
      described_class.class_variable_set(:@@files, {})
      described_class.class_variable_set(:@@pid, pid)
      allow(Prometheus::Client.configuration).to receive(:pid_provider).and_return(-> { pid })
      allow(Process).to receive(:pid).and_return(pid)
    end

    describe 'counter type object initialized' do
      let!(:counter) { described_class.new(:counter, :counter, 'counter', {}) }

      describe 'PID unchanged' do
        it 'initializing gauge MmapValue object type keeps old file data' do
          described_class.new(:gauge, :gauge, 'gauge', {}, :all)
          expect(described_class.class_variable_get(:@@files)).to have_key('counter')
          expect(described_class.class_variable_get(:@@files)).to have_key('gauge_all')
        end
      end

      describe 'PID changed' do
        let(:new_pid) { pid - 1 }
        let(:page_size) { Prometheus::Client::PageSize.page_size }

        before do
          counter.increment
          @old_value = counter.get

          allow(Prometheus::Client.configuration).to receive(:pid_provider).and_return(-> { new_pid })
          allow(Process).to receive(:pid).and_return(new_pid)
        end

        it 'initializing gauge MmapValue object type keeps old file data' do
          described_class.new(:gauge, :gauge, 'gauge', {}, :all)

          expect(described_class.class_variable_get(:@@files)).not_to have_key('counter')
          expect(described_class.class_variable_get(:@@files)).to have_key('gauge_all')
        end

        it 'updates pid' do
          expect { described_class.new(:gauge, :gauge, 'gauge', {}, :all) }
            .to change { described_class.class_variable_get(:@@pid) }.from(pid).to(new_pid)
        end

        it '#increment updates pid' do
          expect { counter.increment }
            .to change { described_class.class_variable_get(:@@pid) }.from(pid).to(new_pid)
        end

        it '#increment updates pid' do
          expect { counter.increment }
            .to change { described_class.class_variable_get(:@@pid) }.from(pid).to(new_pid)
        end

        it '#get updates pid' do
          expect { counter.get }
            .to change { described_class.class_variable_get(:@@pid) }.from(pid).to(new_pid)
        end

        it '#set updates pid' do
          expect { counter.set(1) }
            .to change { described_class.class_variable_get(:@@pid) }.from(pid).to(new_pid)
        end

        it '#set logs an error' do
          counter.set(1)

          allow(counter.instance_variable_get(:@file))
            .to receive(:write_value)
            .and_raise('error writing value')
          expect(Prometheus::Client.logger).to receive(:warn).and_call_original

          counter.set(1)
        end

        it 'reinitialize restores all used file references and resets data' do
          described_class.new(:gauge, :gauge, 'gauge', {}, :all)
          described_class.reinitialize_on_pid_change

          expect(described_class.class_variable_get(:@@files)).to have_key('counter')
          expect(described_class.class_variable_get(:@@files)).to have_key('gauge_all')
          expect(counter.get).not_to eq(@old_value)
        end

        it 'updates strings properly upon memory expansion', :page_size do
          described_class.new(:gauge, :gauge, 'gauge2', { label_1: 'x' * page_size * 2 }, :all)

          # This previously failed on Linux but not on macOS since mmap() may re-allocate the same region.
          ObjectSpace.each_object(String, &:valid_encoding?)
        end
      end

      context 'different label ordering' do
        it 'does not care about label ordering' do
          counter1 = described_class.new(:counter, :counter, 'ordered_counter', { label_1: 'hello', label_2: 'world', label_3: 'baz' }).increment
          counter2 = described_class.new(:counter, :counter, 'ordered_counter', { label_2: 'world', label_3: 'baz', label_1: 'hello' }).increment

          reading_counter = described_class.new(:counter, :counter, 'ordered_counter', { label_3: 'baz', label_1: 'hello', label_2: 'world' })

          expect(reading_counter.get).to eq(2)
        end
      end
    end
  end
end
