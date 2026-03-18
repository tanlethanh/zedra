package main

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"
)

// ErrNotFound is returned when a key is absent.
var ErrNotFound = errors.New("key not found")

// Cache is a thread-safe in-memory cache with TTL.
type Cache[K comparable, V any] struct {
	mu      sync.RWMutex
	entries map[K]entry[V]
	ttl     time.Duration
}

type entry[V any] struct {
	value     V
	expiresAt time.Time
}

func NewCache[K comparable, V any](ttl time.Duration) *Cache[K, V] {
	return &Cache[K, V]{
		entries: make(map[K]entry[V]),
		ttl:     ttl,
	}
}

func (c *Cache[K, V]) Set(key K, value V) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries[key] = entry[V]{
		value:     value,
		expiresAt: time.Now().Add(c.ttl),
	}
}

func (c *Cache[K, V]) Get(key K) (V, error) {
	c.mu.RLock()
	defer c.mu.RUnlock()
	e, ok := c.entries[key]
	if !ok || time.Now().After(e.expiresAt) {
		var zero V
		return zero, ErrNotFound
	}
	return e.value, nil
}

// Worker processes jobs from a channel concurrently.
func Worker(ctx context.Context, jobs <-chan int, results chan<- int, wg *sync.WaitGroup) {
	defer wg.Done()
	for {
		select {
		case <-ctx.Done():
			return
		case j, ok := <-jobs:
			if !ok {
				return
			}
			results <- j * j
		}
	}
}

func main() {
	cache := NewCache[string, int](5 * time.Second)
	cache.Set("answer", 42)

	if v, err := cache.Get("answer"); err == nil {
		fmt.Printf("cached: %d\n", v)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	jobs := make(chan int, 10)
	results := make(chan int, 10)
	var wg sync.WaitGroup

	for i := 0; i < 3; i++ {
		wg.Add(1)
		go Worker(ctx, jobs, results, &wg)
	}

	for i := 1; i <= 9; i++ {
		jobs <- i
	}
	close(jobs)

	go func() {
		wg.Wait()
		close(results)
	}()

	for r := range results {
		fmt.Println(r)
	}
}
