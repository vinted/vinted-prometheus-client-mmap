module SampleMetrics
  def add_simple_metrics(registry)
    counter = registry.counter(:counter, 'counter', { b: 1 })
    counter.increment(a: 1)
    counter.increment(a: 2)
    counter.increment(a: 1, b: 2)
    gauge = registry.gauge(:gauge, 'gauge', {}, :livesum)
    gauge.set({ b: 1 }, 1)
    gauge.set({ b: 2 }, 1)
    gauge_with_pid = registry.gauge(:gauge_with_pid, 'gauge_with_pid', b: 1)
    gauge_with_pid.set({ c: 1 }, 1)
    gauge_with_null_labels = registry.gauge(:gauge_with_null_labels, 'gauge_with_null_labels', { a: nil, b: nil }, :livesum)
    gauge_with_null_labels.set({ a: nil, b: nil }, 1)
    gauge_with_big_value = registry.gauge(:gauge_with_big_value, 'gauge_with_big_value', { a: 0 }, :livesum)
    gauge_with_big_value.set({ a: 12345678901234567 }, 12345678901234567)
    gauge_with_big_value.set({ a: 0.12345678901234567 }, 0.12345678901234567)

    registry.gauge(:gauge_without_measurements, 'gauge_without_measurements', b: 1)
    registry.histogram(:histogram, 'histogram', {}).observe({ a: 1 }, 1)
    registry.summary(:summary, 'summary', a: 1).observe({ b: 1 }, 1)
  end
end
