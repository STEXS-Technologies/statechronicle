//! Shared baseline profile accessor macros.
/// Defines a set of lazy status accessors returning `&'static Status`.
///
/// Each entry `name => "literal"` produces a `pub fn name() -> &'static Status`
/// backed by a `std::sync::OnceLock`. Because `Status::from_static` owns a
/// `String` it is not `const`, so the compile-time literals are materialized
/// once at first use and reused for the crate's lifetime.
macro_rules! status_set {
    ($( $(#[$doc:meta])* $name:ident => $lit:literal; )*) => {
        $(
            $(#[$doc])*
            pub fn $name() -> &'static Status {
                static VALUE: std::sync::OnceLock<Status> = std::sync::OnceLock::new();
                VALUE.get_or_init(|| Status::from_static($lit))
            }
        )*
    };
}

/// Defines a set of lazy operation accessors plus an `all()` array.
///
/// Each entry `name => "literal"` produces a `pub fn name() -> &'static
/// Operation` backed by a `std::sync::OnceLock`, and a `pub fn all() ->
/// &'static [Operation]` returning the full set in declaration order. Because
/// `Operation::from_static` owns a `String` it is not `const`, so the
/// compile-time literals are materialized once at first use and reused for the
/// crate's lifetime.
macro_rules! op_set {
    ($( $(#[$doc:meta])* $name:ident => $lit:literal; )*) => {
        $(
            $(#[$doc])*
            pub fn $name() -> &'static Operation {
                static VALUE: std::sync::OnceLock<Operation> = std::sync::OnceLock::new();
                VALUE.get_or_init(|| Operation::from_static($lit))
            }
        )*
        /// Every operation in this set, in declaration order.
        pub fn all() -> &'static [Operation] {
            static ALL: std::sync::OnceLock<Vec<Operation>> = std::sync::OnceLock::new();
            ALL.get_or_init(|| vec![ $( $name().clone() ),* ])
        }
    };
}

/// Defines a named `&'static [Operation]` slice from member accessors.
///
/// Each member is an accessor function (from `op_set`) returning
/// `&'static Operation`; the produced `name()` returns the cloned set.
macro_rules! op_slice {
    ($( $(#[$doc:meta])* $name:ident => [ $( $member:ident ),* ]; )*) => {
        $(
            $(#[$doc])*
            pub fn $name() -> &'static [Operation] {
                static ALL: std::sync::OnceLock<Vec<Operation>> = std::sync::OnceLock::new();
                ALL.get_or_init(|| vec![ $( $member().clone() ),* ])
            }
        )*
    };
}
