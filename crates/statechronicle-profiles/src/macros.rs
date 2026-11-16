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

/// Defines a native command discriminator and its static operation list together.
///
/// The ordinary `match` is checked for exhaustiveness by Rust. Use explicit
/// variant patterns (no wildcard fallback) to make adding a command require an
/// operation decision. Ignore payloads in these patterns; guards are not
/// accepted. The author still owns the operation/authority policy: Rust cannot
/// prove that a chosen name is appropriate. Duplicate/unreachable patterns are
/// errors. Declaration order determines the list order, not BCS variant indices.
///
/// `StateProfile::operation` delegates to the generated function; pass the
/// generated list to `TypedProfile::new`. Registration still validates names,
/// namespaces and schema. This hosted convenience uses `std::sync::LazyLock`;
/// it changes neither the portable trait nor command serialization.
///
/// ```
/// use statechronicle_profiles::command_operations;
/// use statechronicle_domain::intent::Operation;
/// enum Command { Create { enabled: bool }, Disable }
/// command_operations! {
///     fn operation(command: &Command);
///     static OPERATIONS: Operation;
///     Command::Create { .. } => "configuration.create",
///     Command::Disable => "configuration.disable",
/// }
/// assert_eq!(operation(&Command::Create { enabled: true }), "configuration.create");
/// assert_eq!(OPERATIONS.len(), 2);
/// ```
///
/// A new variant cannot silently be omitted from an explicit mapping:
/// ```compile_fail
/// use statechronicle_profiles::command_operations;
/// use statechronicle_domain::intent::Operation;
/// enum Command { Create, Disable }
/// command_operations! {
///     fn operation(command: &Command);
///     static OPERATIONS: Operation;
///     Command::Create => "configuration.create",
/// }
/// ```
/// Guards are refused:
/// ```compile_fail
/// use statechronicle_profiles::command_operations;
/// use statechronicle_domain::intent::Operation;
/// enum Command { Set(u64) }
/// command_operations! {
///     fn operation(command: &Command);
///     static OPERATIONS: Operation;
///     Command::Set(value) if *value > 0 => "configuration.set",
/// }
/// ```
/// Repeated patterns are errors even when the caller allows ordinary warnings:
/// ```compile_fail
/// use statechronicle_profiles::command_operations;
/// use statechronicle_domain::intent::Operation;
/// enum Command { Create }
/// command_operations! {
///     fn operation(command: &Command);
///     static OPERATIONS: Operation;
///     Command::Create => "configuration.create",
///     Command::Create => "configuration.other",
/// }
/// ```
#[macro_export]
macro_rules! command_operations {
    (
        $(#[$function_meta:meta])* $function_visibility:vis fn $function:ident($command:ident: &$command_type:ty);
        $(#[$list_meta:meta])* $list_visibility:vis static $list:ident: $operation_type:ty;
        $( $pattern:pat => $name:literal ),+ $(,)?
    ) => {
        $(#[$function_meta])*
        #[deny(unreachable_patterns)]
        $function_visibility fn $function($command: &$command_type) -> &'static str {
            match $command { $( $pattern => $name ),+ }
        }
        $(#[$list_meta])*
        $list_visibility static $list: ::std::sync::LazyLock<[
            $operation_type; [$( stringify!($pattern) ),+].len()
        ]> = ::std::sync::LazyLock::new(|| [$( <$operation_type>::from_static($name) ),+]);
    };
}
