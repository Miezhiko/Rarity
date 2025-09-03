#[macro_export]
macro_rules! set {
  ($init:ident = $val:expr, $($lhs:ident = $rhs:expr),*) => {
      let $init = $val;
    $(
      let $lhs = $rhs;
    )*
  };
}

#[macro_export]
macro_rules! setm {
  ($init:ident = $val:expr, $($lhs:ident = $rhs:expr),*) => {
      let mut $init = $val;
    $(
      let mut $lhs = $rhs;
    )*
  };
}
