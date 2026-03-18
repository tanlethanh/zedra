#include <algorithm>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <vector>

// Generic min-heap
template <typename T, typename Compare = std::less<T>>
class Heap {
public:
    void push(T value) {
        data_.push_back(std::move(value));
        sift_up(data_.size() - 1);
    }

    T pop() {
        if (data_.empty()) throw std::underflow_error("heap is empty");
        T top = std::move(data_.front());
        data_.front() = std::move(data_.back());
        data_.pop_back();
        if (!data_.empty()) sift_down(0);
        return top;
    }

    const T& top() const {
        if (data_.empty()) throw std::underflow_error("heap is empty");
        return data_.front();
    }

    bool empty() const noexcept { return data_.empty(); }
    std::size_t size() const noexcept { return data_.size(); }

private:
    std::vector<T> data_;
    Compare cmp_;

    void sift_up(std::size_t i) {
        while (i > 0) {
            std::size_t parent = (i - 1) / 2;
            if (cmp_(data_[i], data_[parent])) {
                std::swap(data_[i], data_[parent]);
                i = parent;
            } else break;
        }
    }

    void sift_down(std::size_t i) {
        while (true) {
            std::size_t left = 2 * i + 1, right = 2 * i + 2, best = i;
            if (left  < data_.size() && cmp_(data_[left],  data_[best])) best = left;
            if (right < data_.size() && cmp_(data_[right], data_[best])) best = right;
            if (best == i) break;
            std::swap(data_[i], data_[best]);
            i = best;
        }
    }
};

// RAII file handle
struct File {
    explicit File(const char *path, const char *mode) : fp_(fopen(path, mode)) {
        if (!fp_) throw std::runtime_error(std::string("cannot open: ") + path);
    }
    ~File() { if (fp_) fclose(fp_); }
    File(const File&) = delete;
    File& operator=(const File&) = delete;

    FILE *get() const noexcept { return fp_; }
private:
    FILE *fp_;
};

int main() {
    Heap<int> h;
    for (int v : {5, 3, 8, 1, 4, 9, 2}) h.push(v);

    std::cout << "sorted: ";
    while (!h.empty()) std::cout << h.pop() << ' ';
    std::cout << '\n';

    // Max-heap via reverse comparator
    Heap<int, std::greater<int>> maxh;
    for (int v : {5, 3, 8, 1}) maxh.push(v);
    std::cout << "max: " << maxh.top() << '\n';

    return 0;
}
