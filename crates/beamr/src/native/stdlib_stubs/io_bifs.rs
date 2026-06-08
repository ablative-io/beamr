//! I/O native stubs for `io` and `io_lib` modules.

use std::sync::atomic::{AtomicI64, Ordering};

use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;
use crate::term::boxed::{Cons, Tuple};
use crate::term::compare;

static NEXT_REPLY_REF: AtomicI64 = AtomicI64::new(1);
const PENDING_IO_KEY_ID: i64 = -9_001;

pub fn bif_io_put_chars_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [chars] = args else {
        return Err(badarg());
    };
    let group_leader = context.group_leader()?;
    send_put_chars(group_leader, *chars, context)
}

pub fn bif_io_put_chars_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [device, chars] = args else {
        return Err(badarg());
    };
    send_put_chars(*device, *chars, context)
}

pub fn bif_io_format_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [format, arguments] = args else {
        return Err(badarg());
    };
    let bytes = format_bytes(*format, *arguments, context)?;
    let chars = context.alloc_binary(&bytes)?;
    bif_io_put_chars_1(&[chars], context)
}

pub fn bif_io_format_3(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [device, format, arguments] = args else {
        return Err(badarg());
    };
    let bytes = format_bytes(*format, *arguments, context)?;
    let chars = context.alloc_binary(&bytes)?;
    bif_io_put_chars_2(&[*device, chars], context)
}

pub fn bif_io_get_line_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [prompt] = args else {
        return Err(badarg());
    };
    let group_leader = context.group_leader()?;
    let request = context.alloc_tuple(&[
        Term::atom(Atom::GET_LINE),
        Term::atom(Atom::UNICODE),
        *prompt,
    ])?;
    send_io_request(group_leader, request, context)
}

pub fn bif_io_setopts_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [_device, _options] = args else {
        return Err(badarg());
    };
    Ok(Term::atom(Atom::OK))
}

pub fn bif_io_lib_format_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [format, arguments] = args else {
        return Err(badarg());
    };
    let bytes = format_bytes(*format, *arguments, context)?;
    context.alloc_binary(&bytes)
}

fn send_put_chars(device: Term, chars: Term, context: &mut ProcessContext) -> Result<Term, Term> {
    let request = context.alloc_tuple(&[
        Term::atom(Atom::PUT_CHARS),
        Term::atom(Atom::UNICODE),
        chars,
    ])?;
    send_io_request(device, request, context)
}

fn send_io_request(
    device: Term,
    request: Term,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    let target_pid = device.as_pid().ok_or_else(badarg)?;
    if let Some((reply_ref, pending_request)) = pending_io(context)? {
        let result = await_io_reply(reply_ref, pending_request, context)?;
        if result != Term::atom(Atom::OK) {
            return Ok(result);
        }
    }

    let from_pid = context.pid().ok_or_else(badarg)?;
    let reply_ref = Term::small_int(NEXT_REPLY_REF.fetch_add(1, Ordering::Relaxed));
    let message = context.alloc_tuple(&[
        Term::atom(Atom::IO_REQUEST),
        Term::pid(from_pid),
        reply_ref,
        request,
    ])?;
    let pending = context.alloc_tuple(&[reply_ref, request])?;
    context.dict_put(pending_io_key(), pending)?;
    let Some(facility) = context.io_protocol_facility() else {
        let _ = context.dict_erase(pending_io_key())?;
        return Err(badarg());
    };
    if !facility.send_io_request(target_pid, message) {
        let _ = context.dict_erase(pending_io_key())?;
        return error_tuple(Atom::NOPROC, context);
    }
    await_io_reply(reply_ref, request, context)
}

fn pending_io_key() -> Term {
    Term::small_int(PENDING_IO_KEY_ID)
}

fn pending_io(context: &mut ProcessContext) -> Result<Option<(Term, Term)>, Term> {
    let pending = context.dict_get(pending_io_key())?;
    if pending == Term::atom(Atom::UNDEFINED) {
        return Ok(None);
    }
    let tuple = Tuple::new(pending).ok_or_else(badarg)?;
    if tuple.arity() != 2 {
        return Err(badarg());
    }
    Ok(Some((
        tuple.get(0).ok_or_else(badarg)?,
        tuple.get(1).ok_or_else(badarg)?,
    )))
}

fn await_io_reply(
    reply_ref: Term,
    request: Term,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    if let Some(result) = take_io_reply(reply_ref, context) {
        let _ = context.dict_erase(pending_io_key())?;
        return normalize_io_result(request, result, context);
    }
    context.request_suspend(None);
    Ok(Term::atom(Atom::OK))
}

fn take_io_reply(reply_ref: Term, context: &mut ProcessContext) -> Option<Term> {
    let facility = context.select_facility()?;
    for index in 0..facility.message_count() {
        let message = facility.peek_message(index)?;
        let tuple = Tuple::new(message)?;
        if tuple.arity() == 3
            && tuple.get(0)? == Term::atom(Atom::IO_REPLY)
            && tuple
                .get(1)
                .is_some_and(|term| compare::exact_eq(term, reply_ref))
        {
            let result = tuple.get(2)?;
            facility.remove_message(index);
            return Some(result);
        }
    }
    None
}

fn normalize_io_result(
    request: Term,
    result: Term,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    let request_tag = Tuple::new(request).and_then(|tuple| tuple.get(0));
    if request_tag == Some(Term::atom(Atom::GET_LINE)) {
        if result == Term::atom(Atom::EOF) {
            return Ok(result);
        }
        let bytes = binary_bytes(result)?;
        return context.alloc_binary(bytes);
    }
    Ok(result)
}

fn error_tuple(reason: Atom, context: &mut ProcessContext) -> Result<Term, Term> {
    context.alloc_tuple(&[Term::atom(Atom::ERROR), Term::atom(reason)])
}

fn format_bytes(format: Term, arguments: Term, context: &ProcessContext) -> Result<Vec<u8>, Term> {
    let format = iodata_bytes(format)?;
    let arguments = list_terms(arguments)?;
    let mut out = Vec::with_capacity(format.len());
    let mut arg_index = 0usize;
    let mut index = 0usize;
    while index < format.len() {
        if format[index] != b'~' {
            out.push(format[index]);
            index += 1;
            continue;
        }
        index += 1;
        let Some(&directive) = format.get(index) else {
            return Err(badarg());
        };
        match directive {
            b's' => {
                let arg = next_arg(&arguments, &mut arg_index)?;
                out.extend_from_slice(binary_bytes(arg)?);
            }
            b'p' | b'w' => {
                let arg = next_arg(&arguments, &mut arg_index)?;
                out.extend_from_slice(render_term(arg, context).as_bytes());
            }
            b'n' => out.push(b'\n'),
            b'~' => out.push(b'~'),
            _ => return Err(badarg()),
        }
        index += 1;
    }
    if arg_index != arguments.len() {
        return Err(badarg());
    }
    Ok(out)
}

fn next_arg(arguments: &[Term], index: &mut usize) -> Result<Term, Term> {
    let term = arguments.get(*index).copied().ok_or_else(badarg)?;
    *index += 1;
    Ok(term)
}

fn iodata_bytes(term: Term) -> Result<Vec<u8>, Term> {
    let mut bytes = Vec::new();
    collect_iodata(term, &mut bytes)?;
    Ok(bytes)
}

fn collect_iodata(term: Term, out: &mut Vec<u8>) -> Result<(), Term> {
    if term.is_nil() {
        return Ok(());
    }
    if let Some(binary) = BinaryRef::new(term) {
        out.extend_from_slice(binary.as_bytes());
        return Ok(());
    }
    if let Some(byte) = term
        .as_small_int()
        .and_then(|value| u8::try_from(value).ok())
    {
        out.push(byte);
        return Ok(());
    }
    let cons = Cons::new(term).ok_or_else(badarg)?;
    collect_iodata(cons.head(), out)?;
    collect_iodata(cons.tail(), out)
}

fn list_terms(term: Term) -> Result<Vec<Term>, Term> {
    let mut terms = Vec::new();
    let mut current = term;
    loop {
        if current.is_nil() {
            return Ok(terms);
        }
        let cons = Cons::new(current).ok_or_else(badarg)?;
        terms.push(cons.head());
        current = cons.tail();
    }
}

fn render_term(term: Term, context: &ProcessContext) -> String {
    if let Some(integer) = term.as_small_int() {
        return integer.to_string();
    }
    if let Some(atom) = term.as_atom() {
        return context
            .atom_table()
            .and_then(|table| table.resolve(atom))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Atom({atom:?})"));
    }
    if term.is_nil() {
        return "[]".to_owned();
    }
    if let Some(pid) = term.as_pid() {
        return format!("<0.{pid}.0>");
    }
    if let Some(binary) = BinaryRef::new(term) {
        return match std::str::from_utf8(binary.as_bytes()) {
            Ok(text) => format!("<<\"{text}\">>"),
            Err(_) => format!("<<{} bytes>>", binary.len()),
        };
    }
    if let Some(tuple) = Tuple::new(term) {
        let mut elements = Vec::with_capacity(tuple.arity());
        for index in 0..tuple.arity() {
            if let Some(element) = tuple.get(index) {
                elements.push(render_term(element, context));
            }
        }
        return format!("{{{}}}", elements.join(", "));
    }
    format!("{term:?}")
}

fn binary_bytes(term: Term) -> Result<&'static [u8], Term> {
    BinaryRef::new(term)
        .map(|binary| binary.as_bytes())
        .ok_or_else(badarg)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}
