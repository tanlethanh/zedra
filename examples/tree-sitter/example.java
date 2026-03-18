import java.util.*;
import java.util.concurrent.*;
import java.util.function.*;
import java.util.stream.*;

/**
 * Generic bounded blocking queue backed by an array.
 */
public class BoundedQueue<T> {
    private final Object[] buffer;
    private int head = 0, tail = 0, size = 0;
    private final int capacity;
    private final Object lock = new Object();

    public BoundedQueue(int capacity) {
        if (capacity <= 0) throw new IllegalArgumentException("capacity must be positive");
        this.capacity = capacity;
        this.buffer   = new Object[capacity];
    }

    public void put(T item) throws InterruptedException {
        synchronized (lock) {
            while (size == capacity) lock.wait();
            buffer[tail] = item;
            tail = (tail + 1) % capacity;
            size++;
            lock.notifyAll();
        }
    }

    @SuppressWarnings("unchecked")
    public T take() throws InterruptedException {
        synchronized (lock) {
            while (size == 0) lock.wait();
            T item = (T) buffer[head];
            buffer[head] = null;
            head = (head + 1) % capacity;
            size--;
            lock.notifyAll();
            return item;
        }
    }

    public int size()     { synchronized (lock) { return size; } }
    public boolean isEmpty() { return size() == 0; }
}

// Result monad
sealed interface Result<T> permits Result.Ok, Result.Err {
    record Ok<T>(T value) implements Result<T> {}
    record Err<T>(String message) implements Result<T> {}

    static <T> Result<T> of(Supplier<T> fn) {
        try {
            return new Ok<>(fn.get());
        } catch (Exception e) {
            return new Err<>(e.getMessage());
        }
    }

    default <U> Result<U> map(Function<T, U> fn) {
        return switch (this) {
            case Ok<T> ok   -> Result.of(() -> fn.apply(ok.value()));
            case Err<T> err -> new Err<>(err.message());
        };
    }
}

public class Example {
    static int fibonacci(int n) {
        if (n <= 1) return n;
        int a = 0, b = 1;
        for (int i = 2; i <= n; i++) {
            int c = a + b;
            a = b;
            b = c;
        }
        return b;
    }

    public static void main(String[] args) throws Exception {
        // Fibonacci stream
        List<Integer> fibs = IntStream.range(0, 10)
            .map(Example::fibonacci)
            .boxed()
            .collect(Collectors.toList());
        System.out.println(fibs);

        // Result monad
        Result<Integer> res = Result.of(() -> Integer.parseInt("42"))
            .map(n -> n * 2);
        System.out.println(res);  // Ok[value=84]

        // Bounded queue with producer/consumer
        BoundedQueue<Integer> queue = new BoundedQueue<>(4);
        var producer = new Thread(() -> {
            try {
                for (int i = 0; i < 8; i++) queue.put(i);
            } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
        });
        var consumer = new Thread(() -> {
            try {
                for (int i = 0; i < 8; i++) System.out.println("got " + queue.take());
            } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
        });
        producer.start(); consumer.start();
        producer.join();  consumer.join();
    }
}
