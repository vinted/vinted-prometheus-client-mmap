module TempMetricsDir
  def temp_metrics_dir
    @temp_metrics_dir ||= Dir.mktmpdir('temp_metrics_dir')
  end

  def cleanup_temp_metrics_dir
    return if @temp_metrics_dir.nil?
    begin
      FileUtils.rm_rf(@temp_metrics_dir)
    rescue StandardError => ex
      puts "Files cleanup caused #{ex}"
    ensure
      @temp_metrics_dir = nil
    end
  end
end
