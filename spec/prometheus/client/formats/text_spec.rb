require 'spec_helper'
require 'prometheus/client/formats/text'
require 'prometheus/client/mmaped_value'

describe Prometheus::Client::Formats::Text do
  context 'single process metrics' do
    let(:value_class) { Prometheus::Client::SimpleValue }

    let(:summary_value) do
      { 0.5 => 4.2, 0.9 => 8.32, 0.99 => 15.3 }.tap do |value|
        allow(value).to receive_messages(sum: 1243.21, total: 93)
      end
    end

    let(:histogram_value) do
      { 10 => 1, 20 => 2, 30 => 2 }.tap do |value|
        allow(value).to receive_messages(sum: 15.2, total: 2)
      end
    end

    let(:registry) do
      metrics = [
        double(
          name: :foo,
          docstring: 'foo description',
          base_labels: { umlauts: 'Björn', utf: '佖佥' },
          type: :counter,
          values: {
            { code: 'red' } => 42,
            { code: 'green' } => 3.14E42,
            { code: 'blue' } => -1.23e-45,
          },
        ),
        double(
          name: :bar,
          docstring: "bar description\nwith newline",
          base_labels: { status: 'success' },
          type: :gauge,
          values: {
            { code: 'pink' } => 15,
          },
        ),
        double(
          name: :baz,
          docstring: 'baz "description" \\escaping',
          base_labels: {},
          type: :counter,
          values: {
            { text: "with \"quotes\", \\escape \n and newline" } => 15,
          },
        ),
        double(
          name: :qux,
          docstring: 'qux description',
          base_labels: { for: 'sake' },
          type: :summary,
          values: {
            { code: '1' } => summary_value,
          },
        ),
        double(
          name: :xuq,
          docstring: 'xuq description',
          base_labels: {},
          type: :histogram,
          values: {
            { code: 'ah' } => histogram_value,
          },
        ),
      ]
      metrics.each do |m|
        m.values.each do |k, v|
          m.values[k] = value_class.new(m.type, m.name, m.name, k)
          m.values[k].set(v)
        end
      end
      double(metrics: metrics)
    end

    describe '.marshal' do
      it 'returns a Text format version 0.0.4 compatible representation' do
        expect(subject.marshal(registry)).to eql <<-'TEXT'.gsub(/^\s+/, '')
          # HELP foo foo description
          # TYPE foo counter
          foo{umlauts="Björn",utf="佖佥",code="red"} 42
          foo{umlauts="Björn",utf="佖佥",code="green"} 3.14e+42
          foo{umlauts="Björn",utf="佖佥",code="blue"} -1.23e-45
          # HELP bar bar description\nwith newline
          # TYPE bar gauge
          bar{status="success",code="pink"} 15
          # HELP baz baz "description" \\escaping
          # TYPE baz counter
          baz{text="with \"quotes\", \\escape \n and newline"} 15
          # HELP qux qux description
          # TYPE qux summary
          qux{for="sake",code="1",quantile="0.5"} 4.2
          qux{for="sake",code="1",quantile="0.9"} 8.32
          qux{for="sake",code="1",quantile="0.99"} 15.3
          qux_sum{for="sake",code="1"} 1243.21
          qux_count{for="sake",code="1"} 93
          # HELP xuq xuq description
          # TYPE xuq histogram
          xuq{code="ah",le="10"} 1
          xuq{code="ah",le="20"} 2
          xuq{code="ah",le="30"} 2
          xuq{code="ah",le="+Inf"} 2
          xuq_sum{code="ah"} 15.2
          xuq_count{code="ah"} 2
        TEXT
      end
    end
  end

  context 'multi process metrics', :temp_metrics_dir do
    let(:registry) { Prometheus::Client::Registry.new }

    before do
      allow(Prometheus::Client.configuration).to receive(:multiprocess_files_dir).and_return(temp_metrics_dir)
      # reset all current metrics
      Prometheus::Client::MmapedValue.class_variable_set(:@@files, {})
    end

    context 'pid provider returns compound ID', :temp_metrics_dir, :sample_metrics do
      before do
        allow(Prometheus::Client.configuration).to receive(:pid_provider).and_return(-> { 'pid_provider_id_1' })
        # Prometheus::Client::MmapedValue.class_variable_set(:@@files, {})
        add_simple_metrics(registry)
      end

      it '.marshal_multiprocess' do
        expect(described_class.marshal_multiprocess(temp_metrics_dir)).to eq <<-'TEXT'.gsub(/^\s+/, '')
          # HELP counter Multiprocess metric
          # TYPE counter counter
          counter{a="1",b="1"} 1
          counter{a="1",b="2"} 1
          counter{a="2",b="1"} 1
          # HELP gauge Multiprocess metric
          # TYPE gauge gauge
          gauge{b="1"} 1
          gauge{b="2"} 1
          # HELP gauge_with_big_value Multiprocess metric
          # TYPE gauge_with_big_value gauge
          gauge_with_big_value{a="0.12345678901234566"} 0.12345678901234566
          gauge_with_big_value{a="12345678901234567"} 12345678901234568
          # HELP gauge_with_null_labels Multiprocess metric
          # TYPE gauge_with_null_labels gauge
          gauge_with_null_labels{a="",b=""} 1
          # HELP gauge_with_pid Multiprocess metric
          # TYPE gauge_with_pid gauge
          gauge_with_pid{b="1",c="1",pid="pid_provider_id_1"} 1
          # HELP histogram Multiprocess metric
          # TYPE histogram histogram
          histogram_bucket{a="1",le="+Inf"} 1
          histogram_bucket{a="1",le="0.005"} 0
          histogram_bucket{a="1",le="0.01"} 0
          histogram_bucket{a="1",le="0.025"} 0
          histogram_bucket{a="1",le="0.05"} 0
          histogram_bucket{a="1",le="0.1"} 0
          histogram_bucket{a="1",le="0.25"} 0
          histogram_bucket{a="1",le="0.5"} 0
          histogram_bucket{a="1",le="1"} 1
          histogram_bucket{a="1",le="10"} 1
          histogram_bucket{a="1",le="2.5"} 1
          histogram_bucket{a="1",le="5"} 1
          histogram_count{a="1"} 1
          histogram_sum{a="1"} 1
          # HELP summary Multiprocess metric
          # TYPE summary summary
          summary_count{a="1",b="1"} 1
          summary_sum{a="1",b="1"} 1
        TEXT
      end
    end

    context 'pid provider returns numerical value', :temp_metrics_dir, :sample_metrics do
      before do
        allow(Prometheus::Client.configuration).to receive(:pid_provider).and_return(-> { -1 })
        add_simple_metrics(registry)
      end

      it '.marshal_multiprocess' do
        expect(described_class.marshal_multiprocess(temp_metrics_dir)).to eq <<-'TEXT'.gsub(/^\s+/, '')
          # HELP counter Multiprocess metric
          # TYPE counter counter
          counter{a="1",b="1"} 1
          counter{a="1",b="2"} 1
          counter{a="2",b="1"} 1
          # HELP gauge Multiprocess metric
          # TYPE gauge gauge
          gauge{b="1"} 1
          gauge{b="2"} 1
          # HELP gauge_with_big_value Multiprocess metric
          # TYPE gauge_with_big_value gauge
          gauge_with_big_value{a="0.12345678901234566"} 0.12345678901234566
          gauge_with_big_value{a="12345678901234567"} 12345678901234568
          # HELP gauge_with_null_labels Multiprocess metric
          # TYPE gauge_with_null_labels gauge
          gauge_with_null_labels{a="",b=""} 1
          # HELP gauge_with_pid Multiprocess metric
          # TYPE gauge_with_pid gauge
          gauge_with_pid{b="1",c="1",pid="-1"} 1
          # HELP histogram Multiprocess metric
          # TYPE histogram histogram
          histogram_bucket{a="1",le="+Inf"} 1
          histogram_bucket{a="1",le="0.005"} 0
          histogram_bucket{a="1",le="0.01"} 0
          histogram_bucket{a="1",le="0.025"} 0
          histogram_bucket{a="1",le="0.05"} 0
          histogram_bucket{a="1",le="0.1"} 0
          histogram_bucket{a="1",le="0.25"} 0
          histogram_bucket{a="1",le="0.5"} 0
          histogram_bucket{a="1",le="1"} 1
          histogram_bucket{a="1",le="10"} 1
          histogram_bucket{a="1",le="2.5"} 1
          histogram_bucket{a="1",le="5"} 1
          histogram_count{a="1"} 1
          histogram_sum{a="1"} 1
          # HELP summary Multiprocess metric
          # TYPE summary summary
          summary_count{a="1",b="1"} 1
          summary_sum{a="1",b="1"} 1
        TEXT
      end
    end

    context 'when OJ is available uses OJ to parse keys' do
      let(:oj) { double(oj) }
      before do
        stub_const 'Oj', oj
        allow(oj).to receive(:load)
      end
    end

    context 'with metric having whitespace and UTF chars', :temp_metrics_dir do
      before do
        registry.gauge(:gauge, "bar description\nwith newline", { umlauts: 'Björn', utf: '佖佥' }, :all).set({ umlauts: 'Björn', utf: '佖佥' }, 1)
      end

      xit '.marshall_multiprocess' do
        expect(described_class.marshal_multiprocess(temp_metrics_dir, use_rust: true)).to eq <<-'TEXT'.gsub(/^\s+/, '')
TODO...
        TEXT
      end
    end
  end
end
