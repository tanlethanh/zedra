# frozen_string_literal: true

require "json"
require "net/http"

# Simple LRU cache backed by a Hash (insertion-ordered in Ruby 1.9+).
class LRUCache
  def initialize(capacity)
    @capacity = capacity
    @store    = {}
  end

  def get(key)
    return nil unless @store.key?(key)
    @store[key] = @store.delete(key) # move to end (most-recently used)
  end

  def put(key, value)
    if @store.key?(key)
      @store.delete(key)
    elsif @store.size >= @capacity
      @store.shift # remove oldest entry
    end
    @store[key] = value
  end

  def size   = @store.size
  def to_h   = @store.dup
end

# Functional-style pipeline helper.
module Pipeline
  refine Object do
    def then_pipe(&block)
      block.call(self)
    end
  end
end

using Pipeline

# Tiny HTTP client returning parsed JSON.
def fetch_json(url)
  uri      = URI.parse(url)
  response = Net::HTTP.get_response(uri)
  raise "HTTP #{response.code}" unless response.is_a?(Net::HTTPSuccess)

  JSON.parse(response.body, symbolize_names: true)
end

# Recursive Fibonacci with memoisation.
def fib(n, memo = {})
  return n if n <= 1
  memo[n] ||= fib(n - 1, memo) + fib(n - 2, memo)
end

if __FILE__ == $PROGRAM_NAME
  cache = LRUCache.new(3)
  %w[a b c d].each_with_index { |k, i| cache.put(k, i) }
  p cache.to_h   # {"b"=>1, "c"=>2, "d"=>3}

  fibs = (0..10).map { |n| fib(n) }
  p fibs

  result = [1, 2, 3, 4, 5]
    .then_pipe { |arr| arr.select(&:odd?) }
    .then_pipe { |arr| arr.map { |x| x ** 2 } }
  p result  # [1, 9, 25]
end
