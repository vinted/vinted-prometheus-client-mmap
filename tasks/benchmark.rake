# frozen_string_literal: true

require 'benchmark/memory'
require 'benchmark/ips'
require 'prometheus/client'

namespace :benchmark do
  task :counter do
    counter = Prometheus::Client::Counter.new(:counter, 'Benchmark counter')
    counter_exemplar = Prometheus::Client::Counter.new(:counter_exemplar, 'Benchmark exemplar counter')

    Benchmark.memory do |x|
      x.report('counter.increment') { counter.increment }
      x.report('counter_exemplar.increment') { counter_exemplar.increment({}, 1, 'trace_id', '123') }

      x.compare!
    end

    Benchmark.ips do |x|
      x.report('counter.increment') { counter.increment }
      x.report('counter_exemplar.increment') { counter_exemplar.increment({}, 1, 'trace_id', '123') }

      x.compare!
    end
  end

  task :histogram do
    histogram = Prometheus::Client::Histogram.new(:histogram, 'Benchmark histogram')
    histogram_exemplar = Prometheus::Client::Histogram.new(:histogram_exemplar, 'Benchmark exemplar histogram')

    Benchmark.memory do |x|
      x.report('histogram.observe') { histogram.observe({}, 1) }
      x.report('histogram_exemplar.observe') { histogram_exemplar.observe({}, 1, 'trace_id', '123') }

      x.compare!
    end

    Benchmark.ips do |x|
      x.report('histogram.observe') { histogram.observe({}, 1) }
      x.report('histogram_exemplar.observe') { histogram_exemplar.observe({}, 1, 'trace_id', '123') }

      x.compare!
    end
  end
end
