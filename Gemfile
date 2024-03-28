source 'https://rubygems.org'

gemspec

def ruby_version?(constraint)
  Gem::Dependency.new('', constraint).match?('', RUBY_VERSION)
end

group :test do
  gem 'oj', '> 3'
  gem 'json', '< 2.0' if ruby_version?('< 2.0')
  gem 'simplecov'
  gem 'rack', '< 2.0' if ruby_version?('< 2.2.2')
  gem 'rack-test'
  gem 'rake'
  gem 'pry'
  gem 'rb_sys', '~> 0.9'
  gem 'rspec'
  gem 'rubocop', ruby_version?('< 2.0') ? '< 0.42' : nil
  gem 'tins', '< 1.7' if ruby_version?('< 2.0')
end

group :benchmark do
  gem 'benchmark-ips'
  gem 'benchmark-memory'
end
