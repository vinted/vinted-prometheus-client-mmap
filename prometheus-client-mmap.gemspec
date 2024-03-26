#encoding: utf-8
$LOAD_PATH.push File.expand_path('../lib', __FILE__)
require 'prometheus/client/version'

Gem::Specification.new do |s|
  s.name              = 'vinted-prometheus-client-mmap'
  s.version           = Prometheus::Client::VERSION
  s.summary           = 'A suite of instrumentation metric primitives ' \
                        'that can be exposed through a web services interface.'
  s.authors           = ['Tobias Schmidt', 'Paweł Chojnacki', 'Stan Hu', 'Will Chandler']
  s.email             = ['backend@vinted.com']
  s.homepage          = 'https://gitlab.com/gitlab-org/prometheus-client-mmap'
  s.license           = 'Apache-2.0'

  s.metadata["allowed_push_host"] = "https://rubygems.org"

  s.files             = `git ls-files README.md .tool-versions lib ext vendor`.split("\n")
  s.require_paths     = ['lib']
  s.extensions        = Dir.glob('{ext/**/extconf.rb}')

  # This C extension uses ObjectSpace::WeakRef with Integer keys (https://bugs.ruby-lang.org/issues/16035)
  s.required_ruby_version = '>= 2.7.0'

  s.add_dependency "rb_sys", "~> 0.9.86"

  s.add_development_dependency 'fuzzbert', '~> 1.0', '>= 1.0.4'
  s.add_development_dependency 'gem_publisher', '~> 1'
  s.add_development_dependency 'pry', '~> 0.12.2'
  s.add_development_dependency "rake-compiler", "~> 1.2.1"
  s.add_development_dependency "rake-compiler-dock", "~> 1.4.0"
  s.add_development_dependency 'ruby-prof', '~> 0.16.2'
end
