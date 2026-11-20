"""Unit tests for budget.py on small synthetic Rust sources.

Run: python3 -I scripts/test_budget.py
"""
import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import budget  # noqa: E402

SRC = '''// fn commented() {
fn real() {
    let s = "} fn fake() {";
    let c = '}';
    let r = r#"}}}"#;
    /* } */
}

fn lifetimes<'a>(x: &'a str) -> &'a str {
    x
}

trait T {
    fn declared(&self);
}

#[cfg(test)]
mod tests {
    fn helper() {
        let _ = 1;
    }
    #[test]
    fn t() {}
}
'''


class AnalyseTests(unittest.TestCase):
    def test_strings_comments_and_chars_do_not_confuse_brace_matching(self):
        _, fns = budget.analyse(SRC)
        self.assertEqual([(n, length) for n, _, length in fns], [("real", 6), ("lifetimes", 3)])

    def test_the_cfg_test_module_is_not_production(self):
        lines, fns = budget.analyse(SRC)
        self.assertEqual(lines, 16)  # lines 1-16: the blank line before #[cfg(test)] is production
        self.assertNotIn("helper", [n for n, _, _ in fns])

    def test_a_cfg_test_item_without_a_body_is_skipped(self):
        lines, _ = budget.analyse("#[cfg(test)]\nuse x::y;\nfn a() {\n}\n")
        self.assertEqual(lines, 2)

    def test_a_cfg_test_function_is_skipped(self):
        lines, fns = budget.analyse("fn a() {}\n#[cfg(test)]\n#[allow(x)]\nfn b() {\n}\n")
        self.assertEqual((lines, [n for n, _, _ in fns]), (1, ["a"]))


class MainTests(unittest.TestCase):
    def run_main(self, *args):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "big.rs").write_text("fn long() {\n" + "    let _ = 1;\n" * 10 + "}\n")
            (Path(d) / "small.rs").write_text("fn ok() {}\n")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                code = budget.main(["--root", d, "--max-file", "5", "--max-fn", "5", *args])
            return code, out.getvalue()

    def test_report_mode_prints_offenders_and_exits_zero(self):
        code, out = self.run_main()
        self.assertEqual(code, 0)
        self.assertIn("big.rs", out)
        self.assertNotIn("small.rs", out)
        self.assertIn("2 files scanned, 2 functions; 1 file offenders, 1 function offenders, 2 in all", out)

    def test_fail_exits_one_with_offenders(self):
        self.assertEqual(self.run_main("--fail")[0], 1)

    def test_max_offenders_is_the_ratchet(self):
        self.assertEqual(self.run_main("--max-offenders", "2")[0], 0)
        self.assertEqual(self.run_main("--max-offenders", "1")[0], 1)


if __name__ == "__main__":
    unittest.main()
