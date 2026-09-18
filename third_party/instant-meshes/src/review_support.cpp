/*
    review_support.cpp -- the definitions the trimmed tree needs and its own
    `main.cpp` used to supply.

    Added by this project (see ../review.patch). Only the compute half of
    Instant Meshes is vendored: the viewer, the command-line front end and the
    mesh I/O are not. `field.cpp` declares `extern int nprocs` for the
    `tbb::task_scheduler_init` its interactive `Optimizer` thread creates, and
    `main.cpp` is where that variable used to live.

    The value is unread — the shim's `task_scheduler_init` is a no-op and the
    bridge drives the solver directly rather than starting an `Optimizer` — but
    `Optimizer::run` is still compiled, so the symbol must resolve.
*/

/// -1 is TBB's "decide for me", which is what the shim's pool does anyway.
int nprocs = -1;
