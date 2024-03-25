require "mkmf"
require "rb_sys/mkmf"

if find_executable('rustc')
  create_rust_makefile("fast_mmaped_file_rs") do |r|
    r.auto_install_rust_toolchain = false

    if enable_config('fail-on-warning')
      r.extra_rustflags = ["-Dwarnings"]
    end

    if enable_config('debug')
      r.profile = :dev
    end

    if enable_config('address-sanitizer')
      r.extra_rustflags = ["-Zsanitizer=address"]
    end

    # `rb_sys/mkmf` passes all arguments after `--` directly to `cargo rustc`.
    # We use this awful hack to keep compatibility with existing flags used by
    # the C implementation.
    trimmed_argv = ARGV.take_while { |arg| arg != "--" }
    ARGV = trimmed_argv
  end
else
  raise 'rustc not found. prometheus-client-mmap now requires Rust.'
end
