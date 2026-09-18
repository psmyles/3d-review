/* config.h — generated for this project from lemon's `config.h.in`.
 *
 * Upstream builds this with CMake's `configure_file`. This tree is compiled by
 * `cc` from `crates/optimize/build.rs`, which has no such step, so the answers
 * are written out once here. They are the same ones Blender's build produces
 * for its own vendored copy: no LP/MIP backend (QuadriFlow uses lemon's
 * header-only graph and flow algorithms and none of its solver bindings), and
 * no threading (lemon's own, which nothing here reaches).
 */

#define LEMON_VERSION "1.3.1"
#define LEMON_HAVE_LONG_LONG 1

/* The solver ids lemon's headers compare LEMON_DEFAULT_LP / _MIP against. They
 * are defined whether or not a backend is; leaving them out is a compile error
 * in `lp.h` rather than a missing feature. */
#define _LEMON_CPLEX 1
#define _LEMON_CLP 2
#define _LEMON_GLPK 3
#define _LEMON_SOPLEX 4
#define _LEMON_CBC 5
