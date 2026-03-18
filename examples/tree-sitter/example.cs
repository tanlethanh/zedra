using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace Zedra.Examples;

// Discriminated union via records
public abstract record Shape
{
    public record Circle(double Radius) : Shape;
    public record Rectangle(double Width, double Height) : Shape;
    public record Triangle(double Base, double Height) : Shape;

    public double Area() => this switch
    {
        Circle c        => Math.PI * c.Radius * c.Radius,
        Rectangle r     => r.Width * r.Height,
        Triangle t      => 0.5 * t.Base * t.Height,
        _               => throw new NotImplementedException()
    };

    public string Describe() =>
        $"{GetType().Name}: area = {Area():F2}";
}

// Generic observable collection
public sealed class ObservableList<T> : IDisposable
{
    private readonly List<T> _items = new();
    private readonly SemaphoreSlim _lock = new(1, 1);

    public event EventHandler<T>? ItemAdded;
    public event EventHandler<T>? ItemRemoved;

    public async Task AddAsync(T item, CancellationToken ct = default)
    {
        await _lock.WaitAsync(ct);
        try
        {
            _items.Add(item);
            ItemAdded?.Invoke(this, item);
        }
        finally { _lock.Release(); }
    }

    public async Task<bool> RemoveAsync(T item, CancellationToken ct = default)
    {
        await _lock.WaitAsync(ct);
        try
        {
            bool removed = _items.Remove(item);
            if (removed) ItemRemoved?.Invoke(this, item);
            return removed;
        }
        finally { _lock.Release(); }
    }

    public IReadOnlyList<T> Snapshot()
    {
        _lock.Wait();
        try { return _items.ToList(); }
        finally { _lock.Release(); }
    }

    public void Dispose() => _lock.Dispose();
}

// Extension methods
public static class EnumerableExtensions
{
    public static IEnumerable<T> Flatten<T>(this IEnumerable<IEnumerable<T>> source) =>
        source.SelectMany(x => x);

    public static IEnumerable<(T Item, int Index)> Indexed<T>(this IEnumerable<T> source) =>
        source.Select((item, i) => (item, i));
}

class Program
{
    static async Task Main()
    {
        Shape[] shapes =
        [
            new Shape.Circle(5),
            new Shape.Rectangle(4, 6),
            new Shape.Triangle(3, 8),
        ];

        foreach (var s in shapes)
            Console.WriteLine(s.Describe());

        using var list = new ObservableList<string>();
        list.ItemAdded += (_, item) => Console.WriteLine($"+ {item}");

        await list.AddAsync("alpha");
        await list.AddAsync("beta");
        await list.AddAsync("gamma");

        var nested = new[] { new[] { 1, 2 }, new[] { 3, 4 }, new[] { 5 } };
        var flat   = nested.Flatten().ToList();
        Console.WriteLine(string.Join(", ", flat));

        foreach (var (item, idx) in flat.Indexed())
            Console.WriteLine($"  [{idx}] = {item}");
    }
}
