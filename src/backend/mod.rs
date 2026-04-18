use core::fmt;
use std::fmt::Write;

pub mod rust;

trait Generate<T> {
	fn write_to(&self, buf: &mut impl Write, ctx: &T) -> fmt::Result;
}
