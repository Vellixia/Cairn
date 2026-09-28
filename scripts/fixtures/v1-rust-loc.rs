//! Rustdoc says #[cfg(test)] { braces } but is production.
const NORMAL: &str = "#[cfg(test)] { braces }";
const RAW: &str = r###"#[cfg(test)] { braces }"###;
// #[cfg(test)] { braces } comment stays production.
/* #[cfg(test)] { braces } block comment stays production. */
#[cfg(test)]
use crate::test_support;
#[cfg(test)]
static TEST_VALUE: &str = "{ guarded static }";
#[cfg(test)]
fn direct_guarded_function() { let braces = "{ string }"; }
#[cfg(test)]
impl Guarded { fn method() { /* { comment } */ } }
#[cfg(test)]
mod guarded_module { const VALUE: &str = r#"{ raw }"#; }
