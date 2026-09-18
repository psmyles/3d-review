/*
    tbb.h -- the slice of Intel TBB the vendored Instant Meshes compute set uses,
    reimplemented over the C++ standard library.

    Added by this project. Instant Meshes `#include <tbb/...>` throughout; this
    directory goes on the compiler's include path *ahead* of anything else, so
    those includes resolve here and not one line of the vendored sources changes.
    Vendoring oneTBB itself would add a second build system, a second library to
    ship, and a dependency far larger than the algorithm it parallelizes.

    Only the members the vendored code actually calls exist. That set was
    discovered by compiling: a member this file is missing is a compile error
    naming it, never a silent behaviour change.

    ## Determinism

    Every reduction chunks the range **by grainsize in index order and joins the
    partial results sequentially**, so its floating-point result is independent of
    how many threads ran it — which is what `parallel_deterministic_reduce`
    promises and what this project needs, since a remesh must produce the same
    bytes on every machine. `parallel_reduce` is implemented identically rather
    than more loosely: the looser version would buy nothing and cost
    reproducibility.

    `parallel_for` is a different matter, and the vendored code's own
    `deterministic` flag does **not** cover it. That flag serializes the passes
    upstream knew were order-sensitive — the edge classification and collapse in
    `extract.cpp`, the graph colouring in `hierarchy.cpp` — but the field solve
    itself (`optimize_orientations` / `optimize_positions` in `field.cpp`) runs
    in parallel either way, on the assumption that the graph colouring makes each
    phase's writes disjoint. Measured: it does not. The same mesh solved at one
    thread, at two and at eight gives three different meshes, and at high thread
    counts two runs in one process can disagree.

    So a caller that needs the same bytes twice opens a [`shim::SerialScope`],
    which makes every `parallel_*` on that thread run in order on that thread.
    `review_remesh_run` opens one for the whole solve whenever the caller asked
    for a reproducible result, which is what makes that setting true rather than
    nearly true. Nothing here second-guesses the vendored flag; this is the
    parallelism the vendored flag never reached.

    ## Threads

    One pool, created on first use, sized from `REVIEW_REMESH_THREADS` when set
    and from the hardware concurrency otherwise. The calling thread joins in, so
    a pool of N threads runs N+1 ways. A `parallel_*` reached from inside the
    pool, or while another one is already running, executes serially rather than
    deadlocking on itself.
*/

#pragma once

#include <algorithm>
#include <atomic>
#include <condition_variable>
#include <cstddef>
#include <cstdlib>
#include <functional>
#include <mutex>
#include <queue>
#include <thread>
#include <utility>
#include <vector>

namespace tbb {

// ---------------------------------------------------------------------------
// Ranges
// ---------------------------------------------------------------------------

/// A half-open `[begin, end)` interval plus the grain size a partitioner should
/// not subdivide below. TBB's own type is splittable; this one is only ever
/// chunked by [`shim::chunks_of`], so the splitting constructor is not needed.
template <typename Value> class blocked_range {
public:
    typedef Value const_iterator;
    typedef std::size_t size_type;

    blocked_range(Value begin, Value end, size_type grainsize = 1)
        : begin_(begin), end_(end), grainsize_(grainsize ? grainsize : 1) {}

    Value begin() const { return begin_; }
    Value end() const { return end_; }
    size_type grainsize() const { return grainsize_; }
    bool empty() const { return !(begin_ < end_); }
    size_type size() const {
        return empty() ? size_type(0) : size_type(end_ - begin_);
    }

private:
    Value begin_;
    Value end_;
    size_type grainsize_;
};

namespace shim {

/// `[begin, end)` cut into consecutive `grainsize`-long pieces. The count
/// depends only on the range, never on the thread count — which is what makes
/// the reductions below reproducible.
template <typename Value>
inline std::vector<blocked_range<Value>> chunks_of(const blocked_range<Value> &range) {
    std::vector<blocked_range<Value>> chunks;
    const std::size_t total = range.size();
    if (total == 0) {
        return chunks;
    }
    const std::size_t grain = range.grainsize();
    const std::size_t count = (total + grain - 1) / grain;
    chunks.reserve(count);
    for (std::size_t chunk = 0; chunk < count; ++chunk) {
        const std::size_t first = chunk * grain;
        const std::size_t last = std::min(first + grain, total);
        chunks.push_back(blocked_range<Value>(
            static_cast<Value>(range.begin() + static_cast<Value>(first)),
            static_cast<Value>(range.begin() + static_cast<Value>(last)), grain));
    }
    return chunks;
}

/// Whether this thread has asked for its parallel work to run in order.
///
/// Thread-local rather than global: the Opt workspace runs its stack on a worker
/// of its own, and one node asking for reproducibility must not quietly
/// serialize anything else the process happens to be doing.
inline bool &serial_here() {
    thread_local bool serial = false;
    return serial;
}

/// Makes every `parallel_*` reached from this thread run in order, for as long
/// as it is alive. Nests: an inner scope that asks for order does not undo it on
/// the way out.
class SerialScope {
public:
    explicit SerialScope(bool enabled)
        : enabled_(enabled), previous_(serial_here()) {
        if (enabled_) {
            serial_here() = true;
        }
    }
    ~SerialScope() {
        if (enabled_) {
            serial_here() = previous_;
        }
    }
    SerialScope(const SerialScope &) = delete;
    SerialScope &operator=(const SerialScope &) = delete;

private:
    bool enabled_;
    bool previous_;
};

/// The worker pool. One instance for the process, built on first use and torn
/// down at exit.
class ThreadPool {
public:
    static ThreadPool &instance() {
        static ThreadPool pool;
        return pool;
    }

    /// Workers plus the calling thread.
    unsigned width() const { return static_cast<unsigned>(workers_.size()) + 1u; }

    /// Run `body(i)` for every `i` in `[0, count)` and return once all of them
    /// have finished. Falls back to running them in order on the calling thread
    /// when this thread asked for order ([`SerialScope`]), or when the pool is
    /// unavailable, already busy, or the caller *is* a worker — a nested
    /// `parallel_for` must not wait on the pool it is running inside.
    void run(std::size_t count, const std::function<void(std::size_t)> &body) {
        if (count == 0) {
            return;
        }
        if (serial_here() || count == 1 || workers_.empty() || is_worker() ||
            !busy_.try_lock()) {
            for (std::size_t index = 0; index < count; ++index) {
                body(index);
            }
            return;
        }
        std::lock_guard<std::mutex> busy(busy_, std::adopt_lock);
        {
            std::lock_guard<std::mutex> lock(mutex_);
            job_ = &body;
            count_ = count;
            next_.store(0, std::memory_order_relaxed);
            pending_ = workers_.size();
            generation_ += 1;
        }
        start_.notify_all();
        consume();
        std::unique_lock<std::mutex> lock(mutex_);
        finished_.wait(lock, [this] { return pending_ == 0; });
        job_ = nullptr;
    }

private:
    ThreadPool() {
        unsigned threads = 0;
        if (const char *setting = std::getenv("REVIEW_REMESH_THREADS")) {
            const long parsed = std::strtol(setting, nullptr, 10);
            threads = parsed > 0 ? static_cast<unsigned>(parsed) : 1u;
        } else {
            threads = std::thread::hardware_concurrency();
        }
        if (threads == 0) {
            threads = 1;
        }
        // The caller is one of them, so the pool holds one fewer.
        workers_.reserve(threads - 1);
        for (unsigned worker = 1; worker < threads; ++worker) {
            workers_.emplace_back([this] { loop(); });
        }
    }

    ~ThreadPool() {
        {
            std::lock_guard<std::mutex> lock(mutex_);
            stop_ = true;
            generation_ += 1;
        }
        start_.notify_all();
        for (std::thread &worker : workers_) {
            if (worker.joinable()) {
                worker.join();
            }
        }
    }

    ThreadPool(const ThreadPool &) = delete;
    ThreadPool &operator=(const ThreadPool &) = delete;

    /// Whether the calling thread belongs to this pool.
    static bool &worker_flag() {
        static thread_local bool flag = false;
        return flag;
    }
    static bool is_worker() { return worker_flag(); }

    /// Claim and run indices until the batch is exhausted.
    void consume() {
        const std::function<void(std::size_t)> *body = job_;
        for (;;) {
            const std::size_t index = next_.fetch_add(1, std::memory_order_relaxed);
            if (index >= count_) {
                return;
            }
            (*body)(index);
        }
    }

    void loop() {
        worker_flag() = true;
        std::size_t seen = 0;
        for (;;) {
            std::unique_lock<std::mutex> lock(mutex_);
            start_.wait(lock, [this, seen] { return stop_ || generation_ != seen; });
            if (stop_) {
                return;
            }
            seen = generation_;
            lock.unlock();
            consume();
            lock.lock();
            if (--pending_ == 0) {
                lock.unlock();
                finished_.notify_one();
            }
        }
    }

    std::vector<std::thread> workers_;
    std::mutex mutex_;
    /// Held for the length of one batch, so a second caller runs serially
    /// instead of corrupting the batch in flight.
    std::mutex busy_;
    std::condition_variable start_;
    std::condition_variable finished_;
    const std::function<void(std::size_t)> *job_ = nullptr;
    std::atomic<std::size_t> next_{0};
    std::size_t count_ = 0;
    std::size_t pending_ = 0;
    std::size_t generation_ = 0;
    bool stop_ = false;
};

} // namespace shim

// ---------------------------------------------------------------------------
// parallel_for
// ---------------------------------------------------------------------------

/// `body(sub)` over every grainsize-long piece of `range`.
template <typename Value, typename Body>
inline void parallel_for(const blocked_range<Value> &range, const Body &body) {
    const std::vector<blocked_range<Value>> chunks = shim::chunks_of(range);
    shim::ThreadPool::instance().run(chunks.size(),
                                     [&](std::size_t chunk) { body(chunks[chunk]); });
}

/// `body(i)` for every `i` in `[first, last)`. Unlike the range form there is no
/// grain size to follow, so the work is cut into a few pieces per thread —
/// enough to balance, few enough that the per-piece cost stays negligible.
template <typename Index, typename Body>
inline void parallel_for(Index first, Index last, const Body &body) {
    if (!(first < last)) {
        return;
    }
    const std::size_t total = static_cast<std::size_t>(last - first);
    const std::size_t want = static_cast<std::size_t>(shim::ThreadPool::instance().width()) * 8u;
    const std::size_t grain = std::max<std::size_t>(1, (total + want - 1) / want);
    parallel_for(blocked_range<Index>(first, last, grain),
                 [&](const blocked_range<Index> &sub) {
                     for (Index index = sub.begin(); index != sub.end(); ++index) {
                         body(index);
                     }
                 });
}

// ---------------------------------------------------------------------------
// Reductions
// ---------------------------------------------------------------------------

/// Chunk-wise map, sequential join in index order.
///
/// The partial results are joined on the calling thread from first chunk to
/// last, so the answer is a pure function of the range and the grain size —
/// `parallel_reduce` and `parallel_deterministic_reduce` are the same function
/// here for that reason.
template <typename Value, typename Identity, typename Map, typename Reduce>
inline auto parallel_reduce(const blocked_range<Value> &range, const Identity &identity,
                            const Map &map, const Reduce &reduce)
    -> decltype(map(range, identity)) {
    typedef decltype(map(range, identity)) Result;

    const std::vector<blocked_range<Value>> chunks = shim::chunks_of(range);
    if (chunks.empty()) {
        return Result(identity);
    }
    std::vector<Result> partial(chunks.size(), Result(identity));
    shim::ThreadPool::instance().run(chunks.size(), [&](std::size_t chunk) {
        partial[chunk] = map(chunks[chunk], Result(identity));
    });

    Result total = partial[0];
    for (std::size_t chunk = 1; chunk < partial.size(); ++chunk) {
        total = reduce(total, partial[chunk]);
    }
    return total;
}

template <typename Value, typename Identity, typename Map, typename Reduce>
inline auto parallel_deterministic_reduce(const blocked_range<Value> &range,
                                          const Identity &identity, const Map &map,
                                          const Reduce &reduce)
    -> decltype(map(range, identity)) {
    return parallel_reduce(range, identity, map, reduce);
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

/// Only ever reached on the vendored code's non-deterministic path (its
/// deterministic one calls a stable sort instead), so an ordinary `std::sort`
/// is the whole of it.
template <typename Iterator, typename Compare>
inline void parallel_sort(Iterator first, Iterator last, const Compare &compare) {
    std::sort(first, last, compare);
}

template <typename Iterator> inline void parallel_sort(Iterator first, Iterator last) {
    std::sort(first, last);
}

// ---------------------------------------------------------------------------
// Mutual exclusion
// ---------------------------------------------------------------------------

/// A non-recursive spin lock. The vendored code takes it for a handful of
/// instructions at a time, which is what a spin lock is for; locking one twice
/// on the same thread hangs, exactly as TBB's does.
class spin_mutex {
public:
    spin_mutex() = default;
    spin_mutex(const spin_mutex &) = delete;
    spin_mutex &operator=(const spin_mutex &) = delete;

    void lock() {
        while (flag_.test_and_set(std::memory_order_acquire)) {
            std::this_thread::yield();
        }
    }

    bool try_lock() { return !flag_.test_and_set(std::memory_order_acquire); }

    void unlock() { flag_.clear(std::memory_order_release); }

    class scoped_lock {
    public:
        scoped_lock() = default;
        explicit scoped_lock(spin_mutex &mutex) : mutex_(&mutex) { mutex.lock(); }
        scoped_lock(const scoped_lock &) = delete;
        scoped_lock &operator=(const scoped_lock &) = delete;
        ~scoped_lock() { release(); }

        void acquire(spin_mutex &mutex) {
            release();
            mutex_ = &mutex;
            mutex.lock();
        }

        void release() {
            if (mutex_ != nullptr) {
                mutex_->unlock();
                mutex_ = nullptr;
            }
        }

    private:
        spin_mutex *mutex_ = nullptr;
    };

private:
    std::atomic_flag flag_ = ATOMIC_FLAG_INIT;
};

// ---------------------------------------------------------------------------
// Concurrent containers
// ---------------------------------------------------------------------------

/// A vector that may be appended to from several threads at once.
///
/// Unlike TBB's segmented original this is a plain `std::vector` behind a mutex,
/// so a reference into it is only stable while nothing is appending. That is
/// what the one caller does: `extract.cpp` fills it in one pass and reads,
/// sorts and iterates it in later ones, never both at once.
template <typename T> class concurrent_vector {
public:
    typedef typename std::vector<T>::iterator iterator;
    typedef typename std::vector<T>::const_iterator const_iterator;
    typedef typename std::vector<T>::size_type size_type;
    typedef T value_type;

    void reserve(size_type capacity) {
        std::lock_guard<std::mutex> lock(mutex_);
        data_.reserve(capacity);
    }

    void push_back(const T &value) {
        std::lock_guard<std::mutex> lock(mutex_);
        data_.push_back(value);
    }

    void push_back(T &&value) {
        std::lock_guard<std::mutex> lock(mutex_);
        data_.push_back(std::move(value));
    }

    void clear() {
        std::lock_guard<std::mutex> lock(mutex_);
        data_.clear();
    }

    size_type size() const { return data_.size(); }
    bool empty() const { return data_.empty(); }

    T &operator[](size_type index) { return data_[index]; }
    const T &operator[](size_type index) const { return data_[index]; }

    iterator begin() { return data_.begin(); }
    iterator end() { return data_.end(); }
    const_iterator begin() const { return data_.begin(); }
    const_iterator end() const { return data_.end(); }

private:
    std::vector<T> data_;
    std::mutex mutex_;
};

/// A priority queue that may be pushed to and popped from concurrently.
template <typename T, typename Compare = std::less<T>> class concurrent_priority_queue {
public:
    void push(const T &value) {
        std::lock_guard<std::mutex> lock(mutex_);
        queue_.push(value);
    }

    bool try_pop(T &destination) {
        std::lock_guard<std::mutex> lock(mutex_);
        if (queue_.empty()) {
            return false;
        }
        destination = queue_.top();
        queue_.pop();
        return true;
    }

    bool empty() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return queue_.empty();
    }

    std::size_t size() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return queue_.size();
    }

    void clear() {
        std::lock_guard<std::mutex> lock(mutex_);
        queue_ = std::priority_queue<T, std::vector<T>, Compare>();
    }

private:
    std::priority_queue<T, std::vector<T>, Compare> queue_;
    mutable std::mutex mutex_;
};

// ---------------------------------------------------------------------------
// Scheduler control
// ---------------------------------------------------------------------------

/// A no-op: the pool above sizes itself and lives for the process. Present
/// because the vendored sources construct one.
class task_scheduler_init {
public:
    static constexpr int automatic = -1;
    static constexpr int deferred = -2;

    explicit task_scheduler_init(int = automatic) {}
    void initialize(int = automatic) {}
    void terminate() {}
    static int default_num_threads() {
        return static_cast<int>(shim::ThreadPool::instance().width());
    }
};

// ---------------------------------------------------------------------------
// The task API (`bvh.cpp`'s hierarchy build)
// ---------------------------------------------------------------------------

class task;

namespace shim {

/// Tag types that make `new (allocate_x()) T(...)` pick the allocating operator
/// below. TBB distinguishes root / continuation / child allocation because its
/// scheduler must; here they differ only in intent.
struct allocate_root_tag {};
struct allocate_continuation_tag {
    task *self;
};
struct allocate_child_tag {
    task *self;
};

/// Tasks allocated since the current root started, and the ones ready to run.
///
/// Thread-local because a build runs entirely on the thread that called
/// [`task::spawn_root_and_wait`]: this shim executes the tree serially (the
/// per-node work inside each task is itself parallel), so there is no second
/// thread to hand a task to.
struct TaskArena {
    std::vector<task *> allocated;
    std::vector<task *> ready;
};

inline TaskArena &arena() {
    static thread_local TaskArena arena;
    return arena;
}

} // namespace shim

/// The slice of TBB's (pre-oneTBB) task interface `bvh.cpp` builds its BVH with:
/// allocate a root, recurse by spawning a child and recycling `this` as the
/// other half, and wait for the tree.
///
/// Executed serially, depth first. The recursion exists to parallelize a build
/// whose leaves are cheap; each task's *own* body already runs its binning and
/// partition through `parallel_reduce` / `parallel_for`, so the pool is busy
/// either way and the tree walk costs a stack instead of a scheduler.
class task {
public:
    virtual ~task() = default;

    /// Returns the task to run next — `this` when recycled, `nullptr` when done.
    virtual task *execute() = 0;

    static shim::allocate_root_tag allocate_root() { return shim::allocate_root_tag{}; }
    shim::allocate_continuation_tag allocate_continuation() {
        return shim::allocate_continuation_tag{this};
    }
    shim::allocate_child_tag allocate_child() { return shim::allocate_child_tag{this}; }

    /// Reference counting drives TBB's scheduler; a serial depth-first walk
    /// needs none of it.
    void set_ref_count(int) {}
    void recycle_as_child_of(task &) {}

    void spawn(task &child) { shim::arena().ready.push_back(&child); }

    /// Run `root` and everything it spawns to completion, then free every task
    /// the run allocated. A run is never nested, so clearing the arena at the
    /// end cannot strand another one's tasks.
    static void spawn_root_and_wait(task &root) {
        shim::TaskArena &arena = shim::arena();
        std::vector<task *> stack;
        stack.push_back(&root);
        while (!stack.empty()) {
            task *current = stack.back();
            stack.pop_back();
            const std::size_t before = arena.ready.size();
            task *next = current->execute();
            for (std::size_t index = before; index < arena.ready.size(); ++index) {
                stack.push_back(arena.ready[index]);
            }
            arena.ready.resize(before);
            if (next != nullptr) {
                stack.push_back(next);
            }
        }
        for (task *allocated : arena.allocated) {
            delete allocated;
        }
        arena.allocated.clear();
        arena.ready.clear();
    }
};

/// A task that does nothing. TBB uses one as the continuation a recycled task
/// re-parents onto; here it is simply never run.
class empty_task : public task {
public:
    task *execute() override { return nullptr; }
};

} // namespace tbb

// The allocating placement-new operators behind `new (allocate_root()) T(...)`.
// Each registers the block so [`tbb::task::spawn_root_and_wait`] can free it;
// the matching `operator delete` runs only if the constructor throws.
inline void *operator new(std::size_t size, tbb::shim::allocate_root_tag) {
    void *memory = ::operator new(size);
    tbb::shim::arena().allocated.push_back(static_cast<tbb::task *>(memory));
    return memory;
}

inline void *operator new(std::size_t size, tbb::shim::allocate_continuation_tag) {
    return ::operator new(size, tbb::shim::allocate_root_tag{});
}

inline void *operator new(std::size_t size, tbb::shim::allocate_child_tag) {
    return ::operator new(size, tbb::shim::allocate_root_tag{});
}

inline void operator delete(void *memory, tbb::shim::allocate_root_tag) {
    ::operator delete(memory);
}

inline void operator delete(void *memory, tbb::shim::allocate_continuation_tag) {
    ::operator delete(memory);
}

inline void operator delete(void *memory, tbb::shim::allocate_child_tag) {
    ::operator delete(memory);
}
