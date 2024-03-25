require 'prometheus/client'
require 'prometheus/client/gauge'
require 'examples/metric_example'

describe Prometheus::Client::Gauge do
  let(:gauge) { Prometheus::Client::Gauge.new(:foo, 'foo description', test: nil) }

  before do
    allow(Prometheus::Client.configuration).to receive(:value_class).and_return(Prometheus::Client::SimpleValue)
  end

  it_behaves_like Prometheus::Client::Metric do
    let(:type) { Float }
  end

  describe '#set' do
    it 'sets a metric value' do
      expect do
        gauge.set({}, 42)
      end.to change { gauge.get }.from(0).to(42)
    end

    it 'sets a metric value for a given label set' do
      expect do
        expect do
          gauge.set({ test: 'value' }, 42)
        end.to(change { gauge.get(test: 'value') }.from(0).to(42))
      end.to_not(change { gauge.get })
    end
  end

  describe '#increment' do
    it 'increments a metric value' do
      gauge.set({}, 1)

      expect do
        gauge.increment({}, 42)
      end.to change { gauge.get }.from(1).to(43)
    end

    it 'sets a metric value for a given label set' do
      gauge.increment({ test: 'value' }, 1)
      expect do
        expect do
          gauge.increment({ test: 'value' }, 42)
        end.to(change { gauge.get(test: 'value') }.from(1).to(43))
      end.to_not(change { gauge.get })
    end
  end

  describe '#decrement' do
    it 'decrements a metric value' do
      gauge.set({}, 10)

      expect do
        gauge.decrement({}, 1)
      end.to change { gauge.get }.from(10).to(9)
    end

    it 'sets a metric value for a given label set' do
      gauge.set({ test: 'value' }, 10)
      expect do
        expect do
          gauge.decrement({ test: 'value' }, 5)
        end.to(change { gauge.get(test: 'value') }.from(10).to(5))
      end.to_not(change { gauge.get })
    end
  end

end
