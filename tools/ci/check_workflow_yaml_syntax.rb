#!/usr/bin/env ruby
# frozen_string_literal: true

require "psych"

patterns = [".github/workflows/*.yml", ".github/workflows/*.yaml"]
paths = patterns.flat_map { |pattern| Dir.glob(pattern) }.uniq.sort

if paths.empty?
  warn "::error::No GitHub Actions workflow files found"
  exit 1
end

failed = false

paths.each do |path|
  begin
    Psych.parse_stream(File.read(path), filename: path)
  rescue Psych::SyntaxError => e
    failed = true
    line = e.line || 1
    column = e.column || 1
    message = e.problem || e.message
    puts "::error file=#{path},line=#{line},col=#{column}::Invalid workflow YAML: #{message}"
  end
end

if failed
  warn "Workflow YAML syntax validation failed."
  exit 1
end

puts "Validated #{paths.length} GitHub Actions workflow YAML files."
