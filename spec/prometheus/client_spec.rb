# encoding: UTF-8

require 'prometheus/client'

describe Prometheus::Client do
  describe '.registry' do
    it 'returns a registry object' do
      expect(described_class.registry).to be_a(described_class::Registry)
    end

    it 'memorizes the returned object' do
      expect(described_class.registry).to eql(described_class.registry)
    end
  end

  context '.reset! and .reinitialize_on_pid_change' do
    let(:metric_name) { :room_temperature_celsius }
    let(:label) { { room: 'kitchen' } }
    let(:value) { 21 }
    let(:gauge) { Prometheus::Client::Gauge.new(metric_name, 'test') }

    before do
      described_class.cleanup!
      described_class.reset! # registering metrics will leak into other specs

      registry = described_class.registry
      gauge.set(label, value)
      registry.register(gauge)

      expect(registry.metrics.count).to eq(1)
      expect(registry.get(metric_name).get(label)).to eq(value)
    end

    describe '.reset!' do
      it 'resets registry and clears existing metrics' do
        described_class.cleanup!
        described_class.reset!

        registry = described_class.registry
        expect(registry.metrics.count).to eq(0)

        registry.register(gauge)
        expect(registry.get(metric_name).get(label)).not_to eq(value)
      end
    end

    describe '.reinitialize_on_pid_change' do
      context 'with force: false' do
        it 'calls `MmapedValue.reinitialize_on_pid_change`' do
          expect(Prometheus::Client::MmapedValue).to receive(:reinitialize_on_pid_change).and_call_original

          described_class.reinitialize_on_pid_change(force: false)
        end
      end

      context 'without explicit :force param' do
        it 'defaults to `false` and calls `MmapedValue.reinitialize_on_pid_change`' do
          expect(Prometheus::Client::MmapedValue).to receive(:reinitialize_on_pid_change).and_call_original

          described_class.reinitialize_on_pid_change
        end
      end

      context 'with force: true' do
        it 'calls `MmapedValue.reset_and_reinitialize`' do
          expect(Prometheus::Client::MmapedValue).to receive(:reset_and_reinitialize).and_call_original

          described_class.reinitialize_on_pid_change(force: true)
        end
      end
    end
  end
end
