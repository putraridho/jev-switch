"""Run: python3 -I scripts/test_move_check.py"""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().with_name("move_check.py")


class MoveCheckTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        self.git("init", "-q")
        self.write("old.rs", "use super::thing;\nfn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\nfn five() {}\n")
        self.write("other.rs", "original\n")
        self.commit()

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, env=self.env, check=True, capture_output=True)

    def write(self, path, content):
        (self.repo / path).write_text(content)

    def commit(self):
        self.git("add", "-A")
        self.git("-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-qm", "fixture")

    def check_move(self, *flags):
        self.commit()
        return subprocess.run([sys.executable, "-I", str(SCRIPT), *flags], cwd=self.repo, env=self.env, text=True, capture_output=True)

    def test_an_item_moved_to_another_file_with_its_callers_rewritten_is_an_item_move(self):
        self.write("old.rs", "use super::thing;\nfn one() {}\nfn three() {}\nfn four() {}\nfn five() {}\n")
        self.write("other.rs", "original\n//! The new home.\nfn two() {}\nlet x = crate::shared::Id::A;\n")
        self.check_move()  # a first commit that adds the caller in its old form is set up below
        self.write("other.rs", "original\n//! The new home.\nfn two() {}\nlet x = crate::backend::Id::A;\n")
        self.commit()
        self.write("other.rs", "original\n//! The new home.\nfn two() {}\nlet x = crate::shared::Id::A;\n")
        self.assertEqual(self.check_move("--items").returncode, 0)

    def test_an_item_changed_on_the_way_is_not_an_item_move(self):
        self.write("old.rs", "use super::thing;\nfn one() {}\nfn three() {}\nfn four() {}\nfn five() {}\n")
        self.write("other.rs", "original\nfn two_changed() {}\n")
        result = self.check_move("--items")
        self.assertEqual(result.returncode, 1)
        self.assertIn("fn two_changed()", result.stdout)

    def test_pure_rename(self):
        self.git("mv", "old.rs", "new.rs")
        self.assertEqual(self.check_move().returncode, 0)

    def test_changed_line_fails(self):
        self.git("mv", "old.rs", "new.rs")
        path = self.repo / "new.rs"
        path.write_text(path.read_text().replace("fn one() {}", "fn changed() {}"))
        result = self.check_move()
        self.assertEqual(result.returncode, 1)
        self.assertIn("fn changed()", result.stdout)

    def test_use_and_visibility_changes_pass(self):
        self.git("mv", "old.rs", "new.rs")
        path = self.repo / "new.rs"
        path.write_text(path.read_text().replace("use super::thing;", "use crate::thing;").replace("fn one()", "pub(crate) fn one()"))
        self.assertEqual(self.check_move().returncode, 0)

    def test_extra_modified_file_fails(self):
        self.git("mv", "old.rs", "new.rs")
        self.write("other.rs", "changed\n")
        result = self.check_move()
        self.assertEqual(result.returncode, 1)
        self.assertIn("other.rs", result.stdout)

    def test_scaffold_addition_allowed_other_addition_fails(self):
        self.write("lib.rs", "anything\n")
        self.write("unexpected.rs", "anything\n")
        result = self.check_move()
        self.assertEqual(result.returncode, 1)
        self.assertIn("lib.rs: not a move: added/deleted", result.stdout)
        self.assertIn("unexpected.rs: not a move: added/deleted", result.stdout)


if __name__ == "__main__":
    unittest.main()
