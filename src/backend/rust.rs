use crate::{File, PrimitiveType, TopLevelStatement, Type};
use heck::ToPascalCase;
use std::fmt::Write;

#[derive(Debug)]
struct Signature {
    args: Vec<Arg>,
    ret: Arg,
}

#[derive(Debug)]
enum Arg {
    Handle,
    HiddenOutMemory,
    Primitive,
    InMemory,
}

fn cast(to: PrimitiveType) -> &'static str {
    match to {
        PrimitiveType::Int => "as isize",
        PrimitiveType::Uint => "as usize",
        PrimitiveType::Bool => "!= 0",
        PrimitiveType::Addr => "as *const u8",
        PrimitiveType::Byte => "as u8",
        PrimitiveType::Proto => "as u128",
    }
}

fn ty_str(to: PrimitiveType) -> &'static str {
    match to {
        PrimitiveType::Int => "isize",
        PrimitiveType::Uint => "usize",
        PrimitiveType::Bool => "bool",
        PrimitiveType::Addr => "*const u8",
        PrimitiveType::Byte => "u8",
        PrimitiveType::Proto => "u128",
    }
}

pub fn process(input: File<'_>) {
    for interface in input.content {
        let TopLevelStatement::Interface(interface) = interface;

        let interface_ident = interface.ident.split(".").map(|s| s.to_pascal_case()).fold(
            String::new(),
            |mut acc, v| {
                acc.push_str(&v);
                acc
            },
        );

        let mut buf_dispatch = format!("pub trait {interface_ident}Dispatch {{\n");
        let mut buf_dispatch_impl =
            format!("impl<T: {interface_ident}> {interface_ident}Dispatch for T {{\n");
        let mut buf_main = format!("pub trait {interface_ident}: {interface_ident}Dispatch + core::marker::Sync {{\n");
        let mut buf_vtable = String::new();

        for func in interface.f {
            let ident = func.name.trim_start_matches("~");
            writeln!(
                &mut buf_dispatch_impl,
                "fn dispatch_{ident}<'a>(&'a self, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> core::pin::Pin<alloc::boxed::Box<dyn 'a + core::marker::Send + core::future::Future<Output = Result<popcorn_server::Result, std::os::popcorn::proto::Error>>>> {{\
                alloc::boxed::Box::pin(async move {{"
            );
            writeln!(
                &mut buf_dispatch,
                "fn dispatch_{ident}<'a>(&'a self, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> core::pin::Pin<alloc::boxed::Box<dyn 'a + core::marker::Send + core::future::Future<Output = Result<popcorn_server::Result, std::os::popcorn::proto::Error>>>>;"
            );
            write!(&mut buf_main, "fn {ident}(&self, handle: isize, ");

            writeln!(
                &mut buf_vtable,
                "map.insert({}u128 | ({} as u128) << 96, unsafe {{ core::mem::transmute(<Self as {interface_ident}Dispatch>::dispatch_{ident} as fn(_, _, _, _, _, _) -> _) }});",
                interface.uid,
                func.num,
            );

            let mut out_arg_count = -1;
            let mut get_out_arg = || {
                out_arg_count += 1;
                out_arg_count
            };
            let mut in_arg_count = -1;
            let mut get_in_arg = || {
                in_arg_count += 1;
                in_arg_count
            };

            writeln!(
                &mut buf_dispatch_impl,
                "\tlet v{} = a{} as isize;",
                get_out_arg(),
                get_in_arg()
            );

            match func.ret {
                Some(Type::Slice(ty)) => {
                    let ptr = get_in_arg();
                    let count = get_in_arg();
                    writeln!(
                        &mut buf_dispatch_impl,
                            "let buf = unsafe {{ &mut *::core::ptr::slice_from_raw_parts_mut::<core::mem::MaybeUninit<{}>>(a{ptr} as _, a{count}) }};",
                        ty_str(ty),
                    );
                    writeln!(
                        &mut buf_dispatch_impl,
                        "\tlet v{} = a{count};",
                        get_out_arg()
                    );
                    writeln!(
                        &mut buf_main,
                        "output_size: usize, ",
                    );
                }
                Some(Type::String) => {
                    let ptr = get_in_arg();
                    let count = get_in_arg();
                    writeln!(
                        &mut buf_dispatch_impl,
                        "let buf = unsafe {{ &mut *::core::ptr::slice_from_raw_parts_mut::<core::mem::MaybeUninit<u8>>(a{ptr} as _, a{count}) }};",
                    );
                    writeln!(
                        &mut buf_dispatch_impl,
                        "\tlet v{} = a{count};",
                        get_out_arg()
                    );
                    writeln!(
                        &mut buf_main,
                        "output_size: usize, ",
                    );
                }
                _ => {}
            }

            for (name, arg) in func.args.args {
                let _ = match arg {
                    Type::Primitive(ty) => {
                        writeln!(
                            &mut buf_dispatch_impl,
                            "\tlet v{} = a{} {};",
                            get_out_arg(),
                            get_in_arg(),
                            cast(ty)
                        );
                        writeln!(&mut buf_main, "{name}: {}, ", ty_str(ty));
                    }
                    Type::Slice(ty) => {
                        writeln!(
                            &mut buf_dispatch_impl,
                            "let v{0} = unsafe {{ &*::core::ptr::slice_from_raw_parts::<{ty}>(a{1} as _, a{2} / size_of::<{ty}>()) }};",
                            get_out_arg(),
                            get_in_arg(),
                            get_in_arg(),
                            ty = ty_str(ty),
                        );
                        writeln!(&mut buf_main, "{name}: &[{}], ", ty_str(ty));
                    }
                    Type::Range(_) => todo!(),
                    Type::String => {
                        writeln!(
                            &mut buf_dispatch_impl,
                            "let v{0} = unsafe {{ &*::core::ptr::slice_from_raw_parts::<u8>(a{1} as _, a{2}) }};
                            let v{0} = str::from_utf8(&*v{0}).map_err(|_| std::os::popcorn::proto::Error::InvalidUtf8)?;",
                            get_out_arg(),
                            get_in_arg(),
                            get_in_arg(),
                        );
                        writeln!(&mut buf_main, "{name}: &str, ");
                    }
                    Type::Handle(_proto) => {
                        writeln!(
                            &mut buf_dispatch_impl,
                            "let v{0} = unsafe {{ <std::os::popcorn::handle::OwnedHandle as std::os::popcorn::handle::FromRawHandle>::from_raw_handle(std::os::popcorn::handle::RawHandle(a{1} as isize)) }};",
                            get_out_arg(),
                            get_in_arg(),
                        );
                        writeln!(&mut buf_main, "{name}: std::os::popcorn::handle::OwnedHandle<()>, ");
                    }
                };
            }

            writeln!(&mut buf_main, ") -> impl core::marker::Send + core::future::Future<Output = ");
            let _ = match func.ret {
                None => writeln!(&mut buf_main, "Result<(), std::os::popcorn::proto::Error>"),
                Some(Type::String) => writeln!(&mut buf_main, "Result<String, std::os::popcorn::proto::Error>"),
                Some(Type::Slice(ty)) => {
                    writeln!(&mut buf_main, "Result<Box<[{}]>, std::os::popcorn::proto::Error>", ty_str(ty))
                }
                Some(Type::Primitive(ty)) => {
                    writeln!(&mut buf_main, "Result<{}, std::os::popcorn::proto::Error>", ty_str(ty))
                }
                Some(Type::Handle(_proto)) => {
                    writeln!(&mut buf_main, "Result<popcorn_server::ReturnHandle, std::os::popcorn::proto::Error>")
                }
                Some(Type::Range(_)) => todo!(),
            };
            writeln!(&mut buf_main, "> where Self: Sized;");

            writeln!(
                &mut buf_dispatch_impl,
                "\tlet ret = <Self as {interface_ident}>::{ident}(self, "
            );
            for v in 0..=out_arg_count {
                writeln!(&mut buf_dispatch_impl, "v{v}, ");
            }
            writeln!(&mut buf_dispatch_impl, ").await?;");

            match func.ret {
                Some(Type::Slice(_) | Type::String) => {
                    writeln!(
                        &mut buf_dispatch_impl,
                        r#"let buf_copy = ::core::cmp::min(buf.len(), ret.len());
		                unsafe {{ ::core::ptr::copy_nonoverlapping(ret.as_ptr(), buf.as_mut_ptr().cast(), buf_copy) }};
		                let ret = popcorn_server::Result::Value(ret.len() as u128);"#,
                    );
                },
		        Some(Type::Handle(_proto)) => {
                    writeln!(
                        &mut buf_dispatch_impl,
                        r#";
		                let ret = popcorn_server::Result::from(ret);"#,
                    );
                },
                Some(Type::Range(_)) => todo!(),
                Some(Type::Primitive(_)) => {
                    writeln!(
                        &mut buf_dispatch_impl,
                        r#";
		                let ret = popcorn_server::Result::Value(ret as u128);"#,
                    );
                },
                None => {},
            }

            if func.ret.is_some() { writeln!(&mut buf_dispatch_impl, "\tOk(ret)"); } else { writeln!(&mut buf_dispatch_impl, "\tOk(popcorn_server::Result::Value(0))"); }
            writeln!(&mut buf_dispatch_impl, "}})\n}}");
        }

        writeln!(
            &mut buf_dispatch_impl,
            "fn dispatch_new_from<'a>(&'a self, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> core::pin::Pin<alloc::boxed::Box<dyn 'a + core::marker::Send + core::future::Future<Output = Result<popcorn_server::Result, std::os::popcorn::proto::Error>>>> {{\
                alloc::boxed::Box::pin(async move {{\
                    let v0 = unsafe {{ &*::core::ptr::slice_from_raw_parts::<u8>(a0 as _, a1) }}; \
                    let v0 = str::from_utf8(&*v0).map_err(|_| std::os::popcorn::proto::Error::InvalidUtf8)?; \
                    let v0 = std::path::Path::new(v0); \
                    let v1 = unsafe {{ <std::os::popcorn::handle::OwnedHandle as std::os::popcorn::handle::FromRawHandle>::from_raw_handle(std::os::popcorn::handle::RawHandle(a2 as isize)) }};\
                    let ret = <Self as {interface_ident}>::new_from(self, v0, v1).await?;\
                    Ok(popcorn_server::Result::from(ret))\
                }})\
            }}"
        );
        writeln!(
            &mut buf_dispatch,
            "fn dispatch_new_from<'a>(&'a self, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> core::pin::Pin<alloc::boxed::Box<dyn 'a + core::marker::Send + core::future::Future<Output = Result<popcorn_server::Result, std::os::popcorn::proto::Error>>>>;"
        );

        writeln!(&mut buf_main, "fn __vtable() -> std::collections::HashMap<u128, for<'a> fn(&'a (), usize, usize, usize, usize, usize) -> core::pin::Pin<alloc::boxed::Box<dyn 'a + core::marker::Send + core::future::Future<Output = Result<popcorn_server::Result, std::os::popcorn::proto::Error>>>>> where Self: Sized {{ let mut map = std::collections::HashMap::new(); map.insert({}u128, unsafe {{ core::mem::transmute(<Self as {interface_ident}Dispatch>::dispatch_new_from as fn(_, _, _, _, _, _) -> _) }}); {buf_vtable} map }}", interface.uid);
        writeln!(&mut buf_main, "fn new_from(&self, endpoint: &std::path::Path, handle: std::os::popcorn::handle::OwnedHandle<()>) -> impl core::marker::Send + core::future::Future<Output = Result<popcorn_server::ReturnHandle, std::os::popcorn::proto::Error>> where Self: Sized;");
        buf_dispatch += "}";
        buf_dispatch_impl += "}";
        buf_main += "}";

        println!("{buf_dispatch}");
        println!("{buf_dispatch_impl}");
        println!("{buf_main}");
        println!("impl std::os::popcorn::proto::Protocol for dyn {interface_ident} {{ const UID: u128 = {}; type Ctor = {interface_ident}Ctor; }}", interface.uid);

        println!("#[repr(C)] pub struct {interface_ident}Ctor {{");
        for (name, ty) in interface.ctor.args {
            let _ = match ty {
                Type::Primitive(ty) => println!("pub {name}: {},", ty_str(ty)),
                _ => unreachable!(),
            };
        }
        println!("}}");
    }
}
