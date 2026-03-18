<?php

declare(strict_types=1);

namespace Zedra\Examples;

/**
 * Generic typed collection with functional helpers.
 *
 * @template T
 */
class Collection
{
    /** @param T[] $items */
    public function __construct(private array $items = []) {}

    /** @param T $item */
    public function push(mixed $item): static
    {
        $clone = clone $this;
        $clone->items[] = $item;
        return $clone;
    }

    /** @return static<T> */
    public function filter(callable $predicate): static
    {
        return new static(array_values(array_filter($this->items, $predicate)));
    }

    /** @return static */
    public function map(callable $transform): static
    {
        return new static(array_map($transform, $this->items));
    }

    public function reduce(callable $reducer, mixed $initial = null): mixed
    {
        return array_reduce($this->items, $reducer, $initial);
    }

    public function first(?callable $predicate = null): mixed
    {
        foreach ($this->items as $item) {
            if ($predicate === null || $predicate($item)) {
                return $item;
            }
        }
        return null;
    }

    public function count(): int
    {
        return count($this->items);
    }

    public function toArray(): array
    {
        return $this->items;
    }
}

// Simple Result type
readonly class Ok
{
    public function __construct(public readonly mixed $value) {}
    public function isOk(): bool { return true; }
}

readonly class Err
{
    public function __construct(public readonly string $message) {}
    public function isOk(): bool { return false; }
}

function tryRun(callable $fn): Ok|Err
{
    try {
        return new Ok($fn());
    } catch (\Throwable $e) {
        return new Err($e->getMessage());
    }
}

// Recursive memoised Fibonacci
function fibonacci(int $n, array &$memo = []): int
{
    if ($n <= 1) return $n;
    if (isset($memo[$n])) return $memo[$n];
    return $memo[$n] = fibonacci($n - 1, $memo) + fibonacci($n - 2, $memo);
}

// --- main ---

$numbers = new Collection(range(1, 10));

$result = $numbers
    ->filter(fn(int $n) => $n % 2 === 0)
    ->map(fn(int $n) => $n ** 2)
    ->reduce(fn(int $carry, int $n) => $carry + $n, 0);

echo "Sum of even squares (1–10): $result\n";  // 220

$memo = [];
$fibs = array_map(fn(int $n) => fibonacci($n, $memo), range(0, 9));
echo 'Fibonacci: ' . implode(', ', $fibs) . "\n";

$parsed = tryRun(fn() => json_decode('{"key": 42}', true, flags: JSON_THROW_ON_ERROR));
if ($parsed->isOk()) {
    echo "Parsed key: " . $parsed->value['key'] . "\n";
}

$bad = tryRun(fn() => throw new \RuntimeException("oops"));
echo "Error: $bad->message\n";
