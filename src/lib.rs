#![feature(never_type)]

mod backend;

use anyhow::Context;
use clap::builder::PossibleValue;
use color_print::{ceprint, ceprintln};
use derive_more::{Display, Error};
use std::fmt::Write;
use std::fmt::{Display, Formatter};
use std::path::{PathBuf, Path};
use clap::ValueEnum;

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum OutputFormat {
    Binary,
    Cpp,
    Rust,
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum OutputTy {
    Client,
    Server,
}

impl OutputFormat {
    fn to_file_extension(&self) -> &'static str {
        match self {
            OutputFormat::Binary => "pipb",
            OutputFormat::Cpp => "hpp",
            OutputFormat::Rust => "rs",
        }
    }
}

impl ValueEnum for OutputFormat {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Binary, Self::Cpp, Self::Rust]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            Self::Binary => Some(
                PossibleValue::new("binary")
                    .alias("bin")
                    .help("output `pipb` file for loading into kernel"),
            ),
            Self::Cpp => Some(
                PossibleValue::new("cpp")
                    .alias("c++")
                    .help("generate C++ bindings"),
            ),
            Self::Rust => Some(
                PossibleValue::new("rust")
                    .alias("rs")
                    .help("generate Rust bindings"),
            ),
        }
    }
}

impl ValueEnum for OutputTy {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Client, Self::Server]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            Self::Client => Some(PossibleValue::new("client").help("generate caller bindings")),
            Self::Server => Some(PossibleValue::new("server").help("generate server bindings")),
        }
    }
}

pub fn process_file(file_path: &Path, format: OutputFormat, ty: OutputTy, output: &PathBuf) -> anyhow::Result<()> {
    ceprintln!("       <g><em>Parsing</em></g> {}", file_path.display());

    let file_data = std::fs::read_to_string(file_path).context("failed to open file")?;

    let mut ctx = ParseContext::new(&file_data, file_path);
    let file = File::parse(&mut ctx).context("failed to parse file")?;
    ceprintln!("        <g><em>Parsed</em></g> {}", file_path.display());

    let out = match (format, ty) {
        (OutputFormat::Rust, OutputTy::Server) => backend::rust::process_server(file),
        _ => todo!(),
    };

    let extension = format.to_file_extension();
    let file_path = PathBuf::clone(output)
        .join(file_path.file_stem().context("invalid filename")?)
        .with_added_extension(extension);
    ceprintln!("    <g><em>Generating</em></g> {}", file_path.display());

    std::fs::write(file_path, out).context("failed to write file")?;

    Ok(())
}

#[derive(Debug)]
struct ParseContext<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
}

enum CommentTy {
    None,
    Single,
    Multi,
}

impl<'tokens> ParseContext<'tokens> {
    fn new(tokens: &'tokens str, filename: &'tokens Path) -> ParseContext<'tokens> {
        let mut vec = vec![];

        let mut token_start = None;
        let mut line = 1;
        let mut column_start = 0;
        let mut i_bytes = 0;

        for c in tokens.chars() {
            'matcher: {
                if matches!(
                    c,
                    '@' | '{' | '}' | ':' | '(' | ')' | ',' | '<' | '>' | '[' | ']'
                ) || c.is_whitespace()
                {
                    if let Some(start) = token_start {
                        if c == '>'
                            && matches!(tokens.get(start..start + 1), Some("-"))
                            && start + 1 == i_bytes
                        {
                            vec.push(Token {
                                ty: TokenTy::Arrow,
                                span: Span {
                                    file: filename,
                                    span_start: (line, token_start.unwrap() - column_start),
                                    source: &tokens[start..=i_bytes],
                                },
                            });
                            token_start = None;
                            break 'matcher;
                        }
                        let ident = &tokens[start..i_bytes];
                        vec.push(Token {
                            ty: TokenTy::from(ident),
                            span: Span {
                                file: filename,
                                span_start: (line, start - column_start),
                                source: &tokens[start..i_bytes],
                            },
                        });
                        token_start = None;
                    }
                    if !c.is_whitespace() {
                        vec.push(Token {
                            ty: TokenTy::from(c),
                            span: Span {
                                file: filename,
                                span_start: (line, i_bytes - column_start),
                                source: &tokens[i_bytes..(i_bytes + c.len_utf8())],
                            },
                        });
                    }
                } else if token_start.is_none() {
                    token_start = Some(i_bytes);
                }
            }

            i_bytes += c.len_utf8();
            if c == '\n' {
                line += 1;
                column_start = i_bytes;
            }
        }

        ParseContext {
            tokens: vec,
            pos: 0,
        }
    }

    fn peek(&self) -> Option<Token<'tokens>> {
        self.tokens.get(self.pos).copied()
    }

    fn next(&mut self) -> Result<Token<'tokens>, ParseError> {
        let ret = self.tokens.get(self.pos).copied().ok_or(ParseError)?;
        self.pos += 1;
        Ok(ret)
    }

    fn span_to_idx(&self, span: Span) -> Option<usize> {
        self.tokens
            .iter()
            .position(|token| token.span.span_start == span.span_start)
    }

    fn tokens_by_line(&self, span: Span) -> (&[Token<'tokens>], usize) {
        let Some(mid) = self.span_to_idx(span) else {
            return (&[], 0);
        };
        let mut start = mid;
        let mut end = mid;
        while start != 0 {
            if self.tokens[start - 1].span.span_start.0 == span.span_start.0 {
                start -= 1;
            } else {
                break;
            }
        }
        while end < self.tokens.len() {
            if self.tokens[end + 1].span.span_start.0 == span.span_start.0 {
                end += 1;
            } else {
                break;
            }
        }
        (&self.tokens[start..=end], mid - start)
    }

    fn expect(&mut self, expected: TokenTyDisc) -> Result<Token<'tokens>, ParseError> {
        let tok = self.next()?;
        if tok.ty.as_disc() == expected {
            return Ok(tok);
        } else {
            self.error_unexpected(&[expected], tok.span)?
        }
    }

    fn expect_one(&mut self, expected: &[TokenTyDisc]) -> Result<Token<'tokens>, ParseError> {
        let tok = self.next()?;
        if expected.contains(&tok.ty.as_disc()) {
            return Ok(tok);
        } else {
            self.error_unexpected(expected, tok.span)?
        }
    }

    fn error_str(&mut self, string: &str, span: Span) -> Result<!, ParseError> {
        ceprintln!("<em><r>error</r>: {}</em>", string);

        let line_fmt = format!("{}", span.span_start.0);
        for _ in 0..line_fmt.len() {
            eprint!(" ");
        }
        ceprintln!("<b>--></b> {span}");
        ceprint!("<b>{line_fmt} |</b> ");

        let line = self.tokens_by_line(span);
        let mut whitespace = 0;
        let mut prev_end = 0;
        for (i, token) in line.0.iter().enumerate() {
            let space_before = token.span.span_start.1 - prev_end;
            if i <= line.1 {
                whitespace += space_before;
            }
            if i < line.1 {
                whitespace += token.span.source.len()
            };
            for _ in 0..space_before {
                eprint!(" ");
            }
            eprint!("{}", token.span.source);
            prev_end = token.span.span_start.1 + token.span.source.len();
        }
        eprintln!();

        for _ in 0..line_fmt.len() {
            eprint!(" ");
        }
        ceprint!(" <b>|</b> ");
        for _ in 0..whitespace {
            eprint!(" ");
        }
        for _ in 0..span.source.len() {
            ceprint!("<r>^</r>");
        }
        eprintln!();
        Err(ParseError)
    }

    fn error_unexpected(&mut self, expected: &[TokenTyDisc], span: Span) -> Result<!, ParseError> {
        let str = if expected.len() == 1 {
            format!("expected {}", expected[0])
        } else {
            let mut s = "expected one of ".to_string();
            for (i, ty) in expected.iter().enumerate() {
                if i == expected.len() - 1 {
                    s.push_str(", or ");
                } else if i != 0 {
                    s.push_str(", ");
                }
                let _ = write!(&mut s, "{ty}");
            }
            s.push('\n');
            s
        };

        self.error_str(&str, span)
    }
}

#[derive(Debug, Copy, Clone)]
struct Token<'a> {
    ty: TokenTy<'a>,
    span: Span<'a>,
}

#[derive(Debug, Copy, Clone)]
struct Span<'a> {
    file: &'a Path,
    span_start: (usize, usize),
    source: &'a str,
}

impl Display for Span<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.file.display(),
            self.span_start.0,
            self.span_start.1 + 1
        )
    }
}

#[derive(Debug, Copy, Clone)]
enum TokenTy<'a> {
    At,
    LParen,
    RParen,
    Comma,
    LBrace,
    RBrace,
    LSquare,
    RSquare,
    LAngle,
    RAngle,
    Colon,
    Ident(&'a str),
    Uid(u128),
    Interface,
    Struct,
    Function,
    Factory,
    Numeric(u32),
    Arrow,
}

impl TokenTy<'_> {
    fn as_disc(&self) -> TokenTyDisc {
        match self {
            Self::At => TokenTyDisc::At,
            Self::LParen => TokenTyDisc::LParen,
            Self::RParen => TokenTyDisc::RParen,
            Self::Comma => TokenTyDisc::Comma,
            Self::LBrace => TokenTyDisc::LBrace,
            Self::RBrace => TokenTyDisc::RBrace,
            Self::LAngle => TokenTyDisc::LAngle,
            Self::RAngle => TokenTyDisc::RAngle,
            Self::Colon => TokenTyDisc::Colon,
            Self::Ident(_) => TokenTyDisc::Ident,
            Self::Uid(_) => TokenTyDisc::Uid,
            Self::Numeric(_) => TokenTyDisc::Numeric,
            Self::Interface => TokenTyDisc::Interface,
            Self::Struct => TokenTyDisc::Struct,
            Self::Function => TokenTyDisc::Function,
            Self::Factory => TokenTyDisc::Factory,
            Self::Arrow => TokenTyDisc::Arrow,
            Self::LSquare => TokenTyDisc::LSquare,
            Self::RSquare => TokenTyDisc::RSquare,
        }
    }
}

#[derive(Debug, Copy, Clone, Display, PartialEq)]
enum TokenTyDisc {
    #[display("`@`")]
    At,
    #[display("`(`")]
    LParen,
    #[display("`)`")]
    RParen,
    #[display("`[`")]
    LSquare,
    #[display("`]`")]
    RSquare,
    #[display("`<`")]
    LAngle,
    #[display("`>`")]
    RAngle,
    #[display("`,`")]
    Comma,
    #[display("`{{`")]
    LBrace,
    #[display("`}}`")]
    RBrace,
    #[display("`:`")]
    Colon,
    #[display("identifier")]
    Ident,
    Uid,
    #[display("`interface`")]
    Interface,
    #[display("`struct`")]
    Struct,
    #[display("`func`")]
    Function,
    #[display("`factory`")]
    Factory,
    Numeric,
    #[display("`->`")]
    Arrow,
}

impl<'a> From<&'a str> for TokenTy<'a> {
    fn from(value: &'a str) -> TokenTy<'a> {
        if let Ok(num) = value.parse::<u32>() {
            return TokenTy::Numeric(num);
        }
        match value {
            "interface" => TokenTy::Interface,
            "struct" => TokenTy::Struct,
            "func" => TokenTy::Function,
            "factory" => TokenTy::Function,
            "->" => TokenTy::Arrow,
            _ => {
                if value.len() == 27
                    && value.as_bytes()[4] == b'-'
                    && value.as_bytes()[9] == b'-'
                    && value.as_bytes()[14] == b'-'
                    && value.as_bytes()[0] != b'-'
                {
                    let mut temp = String::from(&value[0..4]);
                    temp.push_str(&value[5..9]);
                    temp.push_str(&value[10..14]);
                    temp.push_str(&value[15..]);
                    if let Ok(uid) = u128::from_str_radix(&temp, 16) {
                        return TokenTy::Uid(uid);
                    }
                }
                TokenTy::Ident(value)
            }
        }
    }
}

impl From<char> for TokenTy<'static> {
    fn from(value: char) -> TokenTy<'static> {
        match value {
            '@' => TokenTy::At,
            '(' => TokenTy::LParen,
            ')' => TokenTy::RParen,
            ',' => TokenTy::Comma,
            '{' => TokenTy::LBrace,
            '}' => TokenTy::RBrace,
            ':' => TokenTy::Colon,
            '[' => TokenTy::LSquare,
            ']' => TokenTy::RSquare,
            '<' => TokenTy::LAngle,
            '>' => TokenTy::RAngle,
            _ => unimplemented!(),
        }
    }
}

#[derive(Debug, Display, Error)]
struct ParseError;

#[derive(Debug)]
struct File<'ctx> {
    content: Vec<TopLevelStatement<'ctx>>,
}

impl<'ctx> File<'ctx> {
    fn parse(ctx: &mut ParseContext<'ctx>) -> Result<File<'ctx>, ParseError> {
        let mut content = vec![];
        while ctx.peek().is_some() {
            content.push(TopLevelStatement::parse(ctx)?);
        }
        Ok(File { content })
    }
}

#[derive(Debug)]
enum TopLevelStatement<'ctx> {
    Interface(Interface<'ctx>),
}

impl<'ctx> TopLevelStatement<'ctx> {
    fn parse(ctx: &mut ParseContext<'ctx>) -> Result<TopLevelStatement<'ctx>, ParseError> {
        let TokenTy::Interface = ctx.expect(TokenTyDisc::Interface)?.ty else {
            unreachable!()
        };
        Ok(Self::Interface(Interface::parse(ctx)?))
    }
}

#[derive(Debug)]
struct Interface<'ctx> {
    ident: &'ctx str,
    ctor: ArgumentList<'ctx>,
    uid: u128,
    f: Vec<Function<'ctx>>,
}

impl<'ctx> Interface<'ctx> {
    fn parse(ctx: &mut ParseContext<'ctx>) -> Result<Interface<'ctx>, ParseError> {
        let TokenTy::Ident(ident) = ctx.expect(TokenTyDisc::Ident)?.ty else {
            unreachable!()
        };
        let ctor = ArgumentList::parse(ctx, false)?;
        ctx.expect(TokenTyDisc::At)?;
        let TokenTy::Uid(uid) = ctx.expect(TokenTyDisc::Uid)?.ty else {
            unreachable!()
        };
        ctx.expect(TokenTyDisc::LBrace)?;

        let mut f = vec![];

        while let Some(next) = ctx.peek()
            && next.ty.as_disc() != TokenTyDisc::RBrace
        {
            f.push(Function::parse(ctx)?);
        }

        ctx.expect(TokenTyDisc::RBrace)?;
        Ok(Interface {
            ident,
            ctor,
            uid,
            f,
        })
    }
}

#[derive(Debug)]
struct ArgumentList<'ctx> {
    args: Vec<(&'ctx str, Type<'ctx>)>,
}

impl<'ctx> ArgumentList<'ctx> {
    fn parse(ctx: &mut ParseContext<'ctx>, accept_complex: bool) -> Result<ArgumentList<'ctx>, ParseError> {
        let mut args = vec![];

        ctx.expect(TokenTyDisc::LParen)?;

        let mut expect_no_comma = |ctx: &mut ParseContext<'ctx>| match ctx
            .expect_one(&[TokenTyDisc::Ident, TokenTyDisc::RParen])?
            .ty
        {
            TokenTy::RParen => return Ok(true),
            TokenTy::Ident(name) => {
                ctx.expect(TokenTyDisc::Colon)?;
                let ty = if accept_complex {
                    Type::parse(ctx)?
                } else {
                    let Token { ty: TokenTy::Ident(ident), span } = ctx.expect(TokenTyDisc::Ident)? else { unreachable!() };
                    Type::Primitive(PrimitiveType::from(ctx, span, ident)?)
                };
                args.push((name, ty));
                return Ok(false);
            }
            _ => unreachable!(),
        };

        if expect_no_comma(ctx)? {
            return Ok(ArgumentList { args });
        }

        loop {
            match ctx
                .expect_one(&[TokenTyDisc::Comma, TokenTyDisc::RParen])?
                .ty
            {
                TokenTy::RParen => break,
                TokenTy::Comma => {
                    if expect_no_comma(ctx)? {
                        break;
                    }
                }
                _ => unreachable!(),
            }
        }

        return Ok(ArgumentList { args });
    }
}

#[derive(Debug, Copy, Clone)]
enum PrimitiveType {
    Int,
    Uint,
    Bool,
    Addr,
    Byte,
    Proto,

}

impl PrimitiveType {
    fn from(ctx: &mut ParseContext, span: Span, string: &str) -> Result<PrimitiveType, ParseError> {
        Ok(match string {
            "int" => PrimitiveType::Int,
            "uint" => PrimitiveType::Uint,
            "bool" => PrimitiveType::Bool,
            "addr" => PrimitiveType::Addr,
            "byte" => PrimitiveType::Byte,
            "proto" => PrimitiveType::Proto,
            _ => ctx.error_str("expected primitive type", span)?,
        })
    }
}

#[derive(Debug)]
enum Type<'ctx> {
    Primitive(PrimitiveType),
    Slice(PrimitiveType),
    Range(PrimitiveType),
    Handle(&'ctx str),
    String,
}

impl<'ctx> Type<'ctx> {
    fn parse(ctx: &mut ParseContext<'ctx>) -> Result<Type<'ctx>, ParseError> {
        let Token { ty, span, .. } = ctx.expect_one(&[TokenTyDisc::Ident, TokenTyDisc::LSquare])?;
        Ok(match ty {
            TokenTy::Ident("handle") => Type::Handle({
                ctx.expect(TokenTyDisc::LAngle)?;
                let TokenTy::Ident(ident) = ctx.expect(TokenTyDisc::Ident)?.ty else {
                    unreachable!()
                };
                ctx.expect(TokenTyDisc::RAngle)?;
                ident
            }),
            TokenTy::Ident("string") => Type::String,
            TokenTy::Ident("range") => Type::Range({
                ctx.expect(TokenTyDisc::LAngle)?;
                let tok = ctx.expect(TokenTyDisc::Ident)?;
                let TokenTy::Ident(ident) = tok.ty else {
                    unreachable!()
                };
                ctx.expect(TokenTyDisc::RAngle)?;
                PrimitiveType::from(ctx, tok.span, ident)?
            }),
            TokenTy::Ident(ident) => Type::Primitive(PrimitiveType::from(ctx, span, ident)?),
            TokenTy::LSquare => Type::Slice({
                let tok = ctx.expect(TokenTyDisc::Ident)?;
                let TokenTy::Ident(ident) = tok.ty else {
                    unreachable!()
                };
                ctx.expect(TokenTyDisc::RSquare)?;
                PrimitiveType::from(ctx, tok.span, ident)?
            }),
            _ => unreachable!(),
        })
    }
}

#[derive(Debug)]
struct Function<'ctx> {
    args: ArgumentList<'ctx>,
    ret: Option<Type<'ctx>>,
    name: &'ctx str,
    num: u32,
}

impl<'ctx> Function<'ctx> {
    fn parse(ctx: &mut ParseContext<'ctx>) -> Result<Function<'ctx>, ParseError> {
        let tok_ty = ctx.expect_one(&[TokenTyDisc::Function, TokenTyDisc::Factory])?;
        if matches!(tok_ty.ty, TokenTy::Factory) {
            todo!("factory function")
        };
        ctx.expect(TokenTyDisc::At)?;
        let tok_id = ctx.expect(TokenTyDisc::Numeric)?;
        let TokenTy::Numeric(num) = tok_id.ty else {
            unreachable!()
        };
        if num == 0 {
            ctx.error_str("function id must not be zero", tok_id.span)?;
        }
        let TokenTy::Ident(name) = ctx.expect(TokenTyDisc::Ident)?.ty else {
            unreachable!()
        };
        let args = ArgumentList::parse(ctx, true)?;
        let ret = if let Some(TokenTy::Arrow) = ctx.peek().map(|tok| tok.ty) {
            ctx.expect(TokenTyDisc::Arrow)?;
            Some(Type::parse(ctx)?)
        } else {
            None
        };
        Ok(Function {
            args,
            ret,
            name,
            num,
        })
    }
}
