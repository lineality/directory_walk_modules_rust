# directory_walk_modules_rust

Vanilla rust (no third party crates or dependencies, no unsafe code) 
version of directory walking (for which a standard third party crate is 
WalkDir).

There are two versions:
1. minimal no-symlink version, simpler and safter

2. a simlink handling version design to work for posix file systems
and windows file systems, but not (yet) for other systems such as Redox OS.


