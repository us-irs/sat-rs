# Working with Constrained Systems

Software for space systems oftentimes has different requirements than the software for host
systems or servers. Currently, most space systems are considered embedded systems.

For these systems, the computation power and the available heap are the most important resources
which are constrained. This might make completeley heap based memory management schemes which
are oftentimes used on host and server based systems unfeasable. Still, completely forbidding
heap allocations might make software development unnecessarilly difficult, especially in a
time where the OBSW might be running on Linux based systems with 500 MB RAM.

A useful pattern used commonly in space systems is to limit heap allocations to program
initialization time and avoid frequent run-time allocations. This prevents issues like
running out of memory (something event Rust can not protect from) or heap fragmentation.

# Using pre-allocated pool structures

A huge candidate for heap allocations is the TMTC and  handling. TC, TMs and IPC data are all
candidates where the data size might vary greatly. The regular solution for host systems
might be to send around this data as a `Vec<u8>` until it is dropped. `sat-rs` provides
another solution to avoid run-time allocations by pre-allocated static pools.

These pools are split into subpools where each subpool can have different page sizes.
For example, a very small TC pool might look like this:

TODO: Add image

A TC entry inside this pool has a store address which can then be sent around without having
to dynamically allocate memory. The same principle can also be applied to the TM and IPC data.

# Using special crates to prevent smaller allocations

Another common way to use the heap on host systems is using containers like `String` and `Vec<u8>`
to work with data where the size is not known beforehand. The most common solution for embedded
systems is to determine the maximum expected size and then use a pre-allocated `u8` buffer and a
size variable. Alternatively, you can use the following crates for more convenience or a smart
behaviour which at least reduced heap allocations:

1. [`smallvec`](https://docs.rs/smallvec/latest/smallvec/).
2. [`arrayvec`](https://docs.rs/arrayvec/latest/arrayvec/index.html) which also contains an
   [`ArrayString`](https://docs.rs/arrayvec/latest/arrayvec/struct.ArrayString.html) helper type.
3. [`tinyvec`](https://docs.rs/tinyvec/latest/tinyvec/).

