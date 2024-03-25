require 'prometheus/client/counter'
require 'prometheus/client'
require 'examples/metric_example'

describe Prometheus::Client::Counter do
  before do
    allow(Prometheus::Client.configuration).to receive(:multiprocess_files_dir).and_return('tmp/')
  end

  let(:counter) { Prometheus::Client::Counter.new(:foo, 'foo description') }

  it_behaves_like Prometheus::Client::Metric do
    let(:type) { Float }
  end

  describe 'Memory Error tests' do
    it "creating many counters shouldn't cause a SIGBUS" do
      4.times do |j|
        9999.times do |i|
          counter = Prometheus::Client::Counter.new("foo#{j}_z#{i}".to_sym, 'some string')
          counter.increment
        end
        GC.start
      end
    end
  end

  describe '#increment' do
    it 'increments the counter' do
      expect { counter.increment }.to change { counter.get }.by(1)
    end

    it 'increments the counter for a given label set' do
      expect do
        expect do
          counter.increment(test: 'label')
        end.to change { counter.get(test: 'label') }.by(1)
      end.to_not change { counter.get(test: 'other_label') }
    end

    it 'increments the counter by a given value' do
      expect do
        counter.increment({}, 5)
      end.to change { counter.get }.by(5)
    end

    it 'raises an ArgumentError on negative increments' do
      expect do
        counter.increment({}, -1)
      end.to raise_error ArgumentError
    end

    it 'returns the new counter value' do
      expect(counter.increment).to eql(counter.get)
    end

    it 'is thread safe' do
      expect do
        Array.new(10) do
          Thread.new do
            10.times { counter.increment }
          end
        end.each(&:join)
      end.to change { counter.get }.by(100)
    end
  end
end
