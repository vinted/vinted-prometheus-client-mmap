$LOAD_PATH.unshift("./lib")

require 'prometheus/client'
require 'prometheus/client/formats/text.rb'
require 'pp'

prometheus = Prometheus::Client.registry

counter = Prometheus::Client::Counter.new(:mycounter, 'Example counter')
gauge = Prometheus::Client::Gauge.new(:mygauge, 'Example gauge', {}, :livesum)
histogram = Prometheus::Client::Histogram.new(:myhistogram, 'Example histogram', {}, [0, 1, 2])
prometheus.register(counter)
prometheus.register(gauge)
prometheus.register(histogram)

counter.increment({'foo': 'bar'}, 2)
counter.increment({'foo': 'biz'}, 4)
gauge.set({'foo': 'bar'}, 3)
gauge.set({'foo': 'biz'}, 3)
gauge.decrement({'foo': 'bar'}, 1)
histogram.observe({'foo': 'bar'}, 0.5)
histogram.observe({'foo': 'biz'}, 0.5)
histogram.observe({'foo': 'bar'}, 1.5)
histogram.observe({'foo': 'biz'}, 2)

#puts Prometheus::Client::Formats::Text.marshal(prometheus)

puts Prometheus::Client::Formats::Text.marshal_multiprocess
