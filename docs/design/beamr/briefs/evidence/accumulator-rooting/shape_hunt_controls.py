"""Two-arm controls for shape_hunt.py's cfg(test) labeller.

⭐ A GREEN FROM AN INSTRUMENT THAT CANNOT FAIL IS WORTH NOTHING. The labeller
reports "27 production, 5 cfg(test)" and the self-check reports "every file
closed at depth 0" -- neither number means anything until the instrument has
been SHOWN, on this run, to be able to produce the other answer.

So every control below is two-armed: a positive that must be detected and a
negative that must not, or a failure that must actually fire.

  A  fixture shape        the exact R10 fixture: #[cfg(test)] mod in a file
                          source_files() does NOT skip -> must be gated
  A2 hit on the opener    a hit on the SAME LINE as the block's opening brace
                          -> must be gated. This arm FOUND A REAL OFF-BY-ONE:
                          the walk originally gated [opener+1 .. closer], so a
                          hit on the opener line was mislabelled `prod`.
  B  negative arm         byte-identical to A minus the #[cfg(test)] attribute
                          -> must gate NOTHING. Without this arm, a labeller
                          that gates everything scores a perfect A.
  C  self-check fires     unbalanced braces -> final depth != 0, which makes
                          shape_hunt exit 3 rather than emit labels it cannot
                          stand behind. Proves the depth-0 green is MEASURED.
  D  char literals        '{' and '}' are values, not structure
  E  raw strings          r#"{{{ }}}"# is text, not structure
  F  block comments       /* } } } */ is text, not structure
  G  cfg on a function    the attribute need not introduce a `mod`

D/E/F are the ways a brace-counting walk silently loses track. Each would
corrupt the labels WITHOUT tripping the self-check if the offsets happened to
cancel, so they are checked directly rather than trusted to the depth-0 green.

Run:  python3 shape_hunt_controls.py     (rc 0 = all arms behaved)
"""
import pathlib
import sys
import tempfile

HUNT = pathlib.Path(__file__).with_name("shape_hunt.py")

# Load cfg_test_lines WITHOUT running the hunt itself (it walks the whole tree).
_src = HUNT.read_text()
_ns = {}
exec(compile(_src[:_src.index("def source_files()")], str(HUNT), "exec"), _ns)
cfg_test_lines = _ns["cfg_test_lines"]

CASES = [
    ("A_fixture_shape",
     "fn prod() {\n    let a = 1;\n}\n\n#[cfg(test)]\nmod tests {\n"
     "    fn helper() {\n        let b = 2;\n    }\n}\n",
     {6, 7, 8, 9, 10}, 0),
    ("A2_hit_on_opener",
     "#[cfg(test)]\nmod t { let x = c.alloc_cons(a, b); }\n",
     {2}, 0),
    ("B_no_cfg_attr",
     "fn prod() {\n    let a = 1;\n}\n\nmod tests {\n"
     "    fn helper() {\n        let b = 2;\n    }\n}\n",
     set(), 0),
    ("C_unbalanced",
     "fn broken() {\n    let a = 1;\n",
     None, 1),
    ("D_char_literal",
     "fn f() {\n    let c = '{';\n    let d = '}';\n}\n",
     set(), 0),
    ("E_raw_string",
     'fn f() {\n    let s = r#"{{{ x }}}"#;\n}\n',
     set(), 0),
    ("F_block_comment",
     "fn f() {\n    /* } } } */\n    let a = 1;\n}\n",
     set(), 0),
    ("G_cfg_on_fn",
     "#[cfg(test)]\nfn helper() {\n    let b = 2;\n}\nfn prod() { let a = 1; }\n",
     {2, 3, 4}, 0),
]


def main():
    tmp = pathlib.Path(tempfile.mkdtemp())
    failures = []
    for name, body, want_gated, want_depth in CASES:
        path = tmp / f"{name}.rs"
        path.write_text(body)
        gated, depth = cfg_test_lines(path)
        ok = (want_gated is None or gated == want_gated) and depth == want_depth
        if not ok:
            failures.append((name, sorted(gated), depth, want_gated, want_depth))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:20s} "
              f"gated={sorted(gated)} depth={depth}")

    print(f"\n{len(CASES) - len(failures)}/{len(CASES)} arms behaved as specified.")
    for name, got_g, got_d, want_g, want_d in failures:
        print(f"  FAIL {name}: got gated={got_g} depth={got_d}, "
              f"expected gated={want_g} depth={want_d}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
