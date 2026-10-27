"""`tools/harness.py check`: every rule AGENTS.md states is named by a guide in harness.ttl,
and a guide and the AGENTS.md section it cites agree on the words they quote."""

from __future__ import annotations

import builtins
import contextlib
import errno
import importlib.util
import io
import os
import re
import shutil
import sys
import tempfile
import types
import unittest
from collections.abc import Callable
from pathlib import Path, PureWindowsPath
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent

spec = importlib.util.spec_from_file_location("harness", ROOT / "tools" / "harness.py")
harness = importlib.util.module_from_spec(spec)
sys.modules["harness"] = harness
spec.loader.exec_module(harness)

GUIDE = "urn:iki:harness:starter:guide:ask-dax-continue-draft"

WINDOWS_RESERVED = {"CON", "PRN", "AUX", "NUL"} | {
    f"{d}{n}" for d in ("COM", "LPT") for n in range(1, 10)
}


def windows_refuses(path: str | bytes | os.PathLike) -> str | None:
    """The first part of `path` that Windows can't name a file or directory, or None. `path` is
    read as Windows reads it, on every platform. A drive (`C:`) counts as one only at the head
    of a path that is absolute here: a drive with nothing after it but a name is what joining
    an `x:` part makes on Windows (`Path("/tmp") / "x:"` is `x:`), and here `x:/…` would name
    a directory `x:` (ikigai-slack#239)."""
    text = os.fsdecode(path)
    path = PureWindowsPath(text)
    if path.drive and not (path.root and Path(text).is_absolute()):
        return path.drive
    for part in path.parts[1:] if path.anchor else path.parts:
        if part in (".", ".."):
            continue
        if (
            any(c in '<>:"|?*' or ord(c) < 32 for c in part)
            or part.endswith((".", " "))
            or part.split(".")[0].rstrip(" ").upper() in WINDOWS_RESERVED
        ):
            return part
    return None


def make_only_windows_names(test: unittest.TestCase) -> None:
    """For the rest of `test`, refuse to make a file, directory or link Windows can't name, as
    Windows would, so a test that makes one fails here too (ikigai-slack#237).

    It guards the calls Python makes a name through (ikigai-slack#240): `open` and `io.open`
    in a mode that can create (w, x or a), `os.open` with O_CREAT, `os.mkdir`, `os.symlink`,
    `os.link`, `os.rename` and `os.replace`, and on Windows `_winapi.CopyFile2`, which from
    Python 3.12 `shutil.copy2` calls instead of copyfile and open (ikigai-slack#245). Through
    them it guards Path's mkdir, touch, open, write_text, write_bytes, symlink_to, hardlink_to,
    rename, replace, and from 3.14 copy and move (whose Windows copy is CopyFile2 too, read
    from the same module), `os.makedirs`, shutil's copy, copy2, copyfile, move and copytree
    (every name copytree makes), and tempfile. It does not guard os.mknod, os.mkfifo, or a
    name a subprocess makes.

    It refuses as Windows refuses, with an OSError whose winerror is ERROR_INVALID_NAME (123)
    and errno EINVAL, since copy2 reads the winerror of what CopyFile2 raises."""

    def guard(make, made: Callable[[tuple, dict], object]):
        # made: what the call makes, or None when it makes nothing.
        def guarded(*args, **kwargs):
            target = made(args, kwargs)
            if target is not None and not isinstance(target, int):
                part = windows_refuses(target)
                if part is not None:
                    text = os.fsdecode(target)
                    error = OSError(errno.EINVAL, f"Windows can't name {part!r}", text, 123)
                    if not hasattr(error, "winerror"):  # only Windows keeps the fourth argument
                        error.winerror = 123
                    raise error
            return make(*args, **kwargs)

        return guarded

    def arg(args: tuple, kwargs: dict, at: int, name: str, default=None):
        return args[at] if len(args) > at else kwargs.get(name, default)

    def opened(args: tuple, kwargs: dict):
        creates = any(c in "wxa" for c in arg(args, kwargs, 1, "mode", "r"))
        return arg(args, kwargs, 0, "file") if creates else None

    def os_opened(args: tuple, kwargs: dict):
        creates = arg(args, kwargs, 1, "flags", 0) & os.O_CREAT
        return arg(args, kwargs, 0, "path") if creates else None

    # shutil holds _winapi on Windows and None elsewhere; a test may stand one in to model
    # Windows. On Windows it is the module pathlib._os reads CopyFile2 from too.
    winapi = getattr(shutil, "_winapi", None)
    for owner, name, made in (
        (builtins, "open", opened),
        (io, "open", opened),
        (os, "open", os_opened),
        (os, "mkdir", lambda args, kwargs: arg(args, kwargs, 0, "path")),
        *((os, name, lambda args, kwargs: arg(args, kwargs, 1, "dst"))
          for name in ("symlink", "link", "rename", "replace")),
        *([(winapi, "CopyFile2", lambda args, kwargs: arg(args, kwargs, 1, "new_file_name"))]
          if hasattr(winapi, "CopyFile2") else []),
    ):
        patcher = mock.patch.object(owner, name, guard(getattr(owner, name), made))
        patcher.start()
        test.addCleanup(patcher.stop)


def symlink_or_skip(test: unittest.TestCase, link: Path, target: Path) -> None:
    """Make `link` a symlink to `target`, or skip `test` (or its subtest) where the platform
    refuses: Windows without Developer Mode or administrator rights raises WinError 1314,
    ERROR_PRIVILEGE_NOT_HELD (ikigai-slack#241). Any other error is raised."""
    try:
        link.symlink_to(target)
    except OSError as error:
        if getattr(error, "winerror", None) != 1314:
            raise
        test.skipTest(
            f"this platform refuses a symlink (WinError 1314: needs Developer Mode or "
            f"administrator rights): {error}"
        )


def absolute_paths(outside: Path, inside: Path, name: str) -> tuple[str, str, str]:
    """Absolute paths a test writes as a statedIn or an enforcedAt, spelled with /: `name` in
    `outside`/x, `inside` itself, and `name` under a directory `x:` of `outside`. The last is
    built as a string: joined as a Path, Windows reads `x:` as a drive and drops `outside`
    (ikigai-slack#239)."""
    return (
        (outside / "x" / name).as_posix(),
        inside.as_posix(),
        f"{outside.as_posix()}/x:/{name}",
    )


class WindowsNamesTest(unittest.TestCase):
    """The guard the tests below make files under: a name Windows refuses is refused here."""

    def test_a_name_windows_refuses_is_refused(self) -> None:
        for part in (
            *("x:", "a<b", "a>b", 'a"b', "a|b", "a?b", "a*b", "a\x01b", "x.", "x "),
            *("CON", "nul.txt", "com1", "LPT9.md"),
        ):
            with self.subTest(part=part):
                # A string, not a join: Windows joins `x:` as a drive (ikigai-slack#239).
                self.assertEqual(part, windows_refuses(Path(f"/tmp/{part}/ci.yml")))

    def test_a_name_windows_takes_is_taken(self) -> None:
        for path in (
            *("/tmp/x/ci.yml", "../x/RULES.md", "./AGENTS.md", ".github/workflows/ci.yml"),
            *("CONSOLE.md", "COM10"),
        ):
            with self.subTest(path=path):
                self.assertIsNone(windows_refuses(path))

    def test_the_guard_refuses_before_it_makes(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        make_only_windows_names(self)
        # A string, not a join: on Windows `Path(tmp.name) / "x:"` is drive X: (ikigai-slack#239).
        bad = Path(f"{Path(tmp.name).as_posix()}/x:")
        with self.assertRaises(OSError):
            bad.mkdir()
        self.assertFalse(bad.exists())
        (Path(tmp.name) / "x").mkdir()
        self.assertTrue((Path(tmp.name) / "x").is_dir())

    def test_the_names_are_read_as_windows_reads_them(self) -> None:
        # ikigai-slack#239: on Windows, Path is WindowsPath, which joins and splits as
        # PureWindowsPath does. Nothing here runs on Windows, so model it.
        with mock.patch.object(sys.modules[__name__], "Path", PureWindowsPath):
            self.test_a_name_windows_refuses_is_refused()
            self.test_a_name_windows_takes_is_taken()

    def test_a_drive_a_join_made_is_refused(self) -> None:
        # ikigai-slack#239: Windows reads a joined `x:` as a drive, which drops what came
        # before it. The guard refuses what that join makes, on every platform.
        for path in (PureWindowsPath("/tmp") / "x:" / "ci.yml", "x:ci.yml", "x:"):
            with self.subTest(path=str(path)):
                self.assertEqual("x:", windows_refuses(path))

    def test_an_absolute_path_is_absolute_on_windows(self) -> None:
        # ikigai-slack#239: on Windows the third of these was the drive-relative `x:ci.yml`.
        outside = PureWindowsPath("C:\\Users\\me\\AppData\\Local\\Temp\\tmp1")
        paths = absolute_paths(outside, outside / "repo" / "AGENTS.md", "ci.yml")
        for path in paths:
            with self.subTest(path=path):
                self.assertTrue(path.startswith("C:/Users/me/AppData/Local/Temp/tmp1/"))
                self.assertTrue(PureWindowsPath(path).is_absolute())
        with mock.patch.object(sys.modules[__name__], "Path", PureWindowsPath):
            self.assertEqual("x:", windows_refuses(paths[2]))

    def test_every_way_a_test_makes_a_name_is_guarded(self) -> None:
        # ikigai-slack#240: each of these made `a:b` here with no error, so a test that made
        # one passed here and failed on Windows.
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        at = Path(tmp.name)
        (at / "ok").write_text("ok", encoding="utf-8")
        (at / "tree").mkdir()
        if os.name != "nt":  # Windows can't hold a bad name in a tree to copy, so has none to guard
            Path(f"{at.as_posix()}/tree/a:b").write_text("x", encoding="utf-8")
        make_only_windows_names(self)
        # A string, not a join: on Windows `at / "a:b"` is `b` on drive A: (ikigai-slack#239).
        bad = Path(f"{at.as_posix()}/a:b")
        with mock.patch.object(sys.modules[__name__], "Path", PureWindowsPath):
            self.assertEqual("a:b", windows_refuses("C:/Users/me/tmp1/a:b"))
        makers = {
            "Path.mkdir": lambda: bad.mkdir(),
            "Path.touch": lambda: bad.touch(),
            "Path.write_text": lambda: bad.write_text("x", encoding="utf-8"),
            "Path.write_bytes": lambda: bad.write_bytes(b"x"),
            "Path.open": lambda: bad.open("w").close(),
            "Path.symlink_to": lambda: bad.symlink_to(at / "ok"),
            "Path.hardlink_to": lambda: bad.hardlink_to(at / "ok"),
            "Path.rename": lambda: (at / "ok").rename(bad),
            "Path.replace": lambda: (at / "ok").replace(bad),
            "open": lambda: open(bad, "w").close(),
            "open, x": lambda: open(bad, "x").close(),
            "open, a": lambda: open(bad, "a").close(),
            "os.open": lambda: os.close(os.open(bad, os.O_WRONLY | os.O_CREAT)),
            "os.mkdir": lambda: os.mkdir(bad),
            "os.makedirs": lambda: os.makedirs(bad / "c"),
            "os.rename": lambda: os.rename(at / "ok", bad),
            "os.replace": lambda: os.replace(at / "ok", bad),
            "os.symlink": lambda: os.symlink(at / "ok", bad),
            "os.link": lambda: os.link(at / "ok", bad),
            "shutil.copy": lambda: shutil.copy(at / "ok", bad),
            "shutil.copy2": lambda: shutil.copy2(at / "ok", bad),
            "shutil.copyfile": lambda: shutil.copyfile(at / "ok", bad),
            "shutil.move": lambda: shutil.move(at / "ok", bad),
            "shutil.copytree": lambda: shutil.copytree(at / "tree", bad),
            "tempfile.mkdtemp": lambda: tempfile.mkdtemp(dir=at, prefix="a:"),
        }
        if os.name != "nt":
            makers["shutil.copytree, a name inside"] = lambda: shutil.copytree(at / "tree", at / "t")
        if hasattr(Path, "copy"):  # Python 3.14
            makers["Path.copy"] = lambda: (at / "ok").copy(bad)
            makers["Path.move"] = lambda: (at / "ok").move(bad)
            if os.name != "nt":
                makers["Path.copy, a name inside"] = lambda: (at / "tree").copy(at / "t")
        for maker, make in makers.items():
            with self.subTest(maker=maker):
                try:
                    with self.assertRaises(OSError):
                        make()
                finally:
                    made = [p for p in at.iterdir() if ":" in p.name] + list(at.glob("t/*"))
                    for p in made:
                        shutil.rmtree(p) if p.is_dir() and not p.is_symlink() else p.unlink()
                    moved = not (at / "ok").exists()
                    if moved:
                        (at / "ok").write_text("ok", encoding="utf-8")
                self.assertEqual([], [p.relative_to(at).as_posix() for p in made])
                self.assertFalse(moved, "ok was moved")

    @unittest.skipIf(os.name == "nt", "Windows has the real CopyFile2; this models it elsewhere")
    def test_a_copy_windows_makes_with_copyfile2_is_guarded(self) -> None:
        # ikigai-slack#245: on Windows, from Python 3.12, shutil.copy2 calls
        # _winapi.CopyFile2(src, dst, flags) and returns before copyfile or open, so the guard
        # on open never saw the name. copytree and move copy files with copy2, and from 3.14
        # Path.copy and Path.move copy with the same CopyFile2 (pathlib._os.copyfile2). Nothing
        # here runs on Windows, so _winapi is modelled, as shutil sees it: shutil reads
        # `_winapi.CopyFile2` when copy2 runs, from the module it imported.
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        at = Path(tmp.name)
        (at / "ok").write_text("ok", encoding="utf-8")
        (at / "tree").mkdir()
        Path(f"{at.as_posix()}/tree/a:b").write_text("x", encoding="utf-8")
        copied: list[str] = []
        winapi = types.ModuleType("_winapi")
        winapi.COPY_FILE_ALLOW_DECRYPTED_DESTINATION = 0x8
        winapi.COPY_FILE_COPY_SYMLINK = 0x800
        winapi.ERROR_PRIVILEGE_NOT_HELD = 1314
        winapi.ERROR_ACCESS_DENIED = 5

        def copy_file_2(src: str, dst: str, flags: int, progress_routine=None) -> None:
            copied.append(dst)  # Windows would make dst here, as a file or an a:b stream
            shutil.copyfile(src, dst)

        winapi.CopyFile2 = copy_file_2
        with mock.patch.object(shutil, "_winapi", winapi):
            make_only_windows_names(self)
            bad = Path(f"{at.as_posix()}/a:b")
            makers = {
                "shutil.copy2": lambda: shutil.copy2(at / "ok", bad),
                "shutil.copytree, a name inside": lambda: shutil.copytree(at / "tree", at / "t"),
                "shutil.move, across devices": lambda: shutil.move(at / "ok", bad),
            }
            across = OSError(errno.EXDEV, "Invalid cross-device link")  # so move copies with copy2
            with mock.patch.object(os, "rename", side_effect=across):
                for maker, make in makers.items():
                    with self.subTest(maker=maker):
                        with self.assertRaises(OSError):
                            make()
                        self.assertEqual([], copied, "CopyFile2 was called with a name Windows refuses")
                        self.assertEqual(["ok", "tree"], sorted(p.name for p in at.iterdir() if p.name != "t"))
                        self.assertEqual([], list(at.glob("t/*")))
            # A name Windows takes still goes through CopyFile2, so the guard changes nothing else.
            self.assertEqual(at / "fine", Path(shutil.copy2(at / "ok", at / "fine")))
            self.assertEqual([str(at / "fine")], copied)


class HarnessCheckTest(unittest.TestCase):
    def test_check_finds_nothing_wrong(self) -> None:
        problems = harness.check(ROOT)
        self.assertEqual([], problems, "\n".join(problems))


class GuideTextDriftTest(unittest.TestCase):
    """Change one copy of a rule, the guide's or AGENTS.md's, and the check fails."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name) / "repo"
        shutil.copytree(ROOT, self.repo, ignore=shutil.ignore_patterns(".git", "target"))
        self.assertEqual([], harness.check(self.repo), "the copy agrees before it is changed")
        make_only_windows_names(self)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def change(self, name: str, old: str, new: str) -> None:
        path = self.repo / name
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new), encoding="utf-8")

    def drift(self) -> list[str]:
        return [p for p in harness.check(self.repo) if p.startswith(GUIDE)]

    def test_a_guide_that_quotes_words_its_section_lacks_fails(self) -> None:
        # The reproduction of ikigai-slack#169: harness.ttl's copy of the rule drifts.
        self.change(
            ".gonk/harness/harness.ttl",
            "begin the new draft's description with `Supersedes PR #N`.",
            "begin the new draft's description with `Replaces pull request N`.",
        )
        problems = self.drift()
        self.assertEqual(1, len(problems), "\n".join(harness.check(self.repo)))
        self.assertIn("`Replaces pull request N`", problems[0])

    def test_a_section_that_loses_words_its_guide_quotes_fails(self) -> None:
        # AGENTS.md's copy drifts the other way.
        self.change(
            "AGENTS.md",
            "description with `Supersedes PR #N`.",
            "description with `Replaces PR #N`.",
        )
        problems = self.drift()
        self.assertEqual(1, len(problems), "\n".join(harness.check(self.repo)))
        self.assertIn("`Supersedes PR #N`", problems[0])

    def test_a_guide_stated_in_a_file_that_is_not_here_fails(self) -> None:
        self.change(
            ".gonk/harness/harness.ttl",
            'harness:statedIn "AGENTS.md" ; harness:section "Ask Dax, don\'t guess" ;\n'
            "    harness:direction harness:feedforward ; harness:execution harness:inferential ;\n"
            "    harness:stage harness:runtime ; harness:category harness:authoritySafety .\n"
            "\nguide:ask-dax-draft-pr",
            'harness:statedIn "NOWHERE.md" ; harness:section "Ask Dax, don\'t guess" ;\n'
            "    harness:direction harness:feedforward ; harness:execution harness:inferential ;\n"
            "    harness:stage harness:runtime ; harness:category harness:authoritySafety .\n"
            "\nguide:ask-dax-draft-pr",
        )
        problems = [
            p
            for p in harness.check(self.repo)
            if p.startswith("urn:iki:harness:starter:guide:ask-dax-dont-guess")
        ]
        self.assertEqual(
            [
                "urn:iki:harness:starter:guide:ask-dax-dont-guess: statedIn 'NOWHERE.md', "
                "which is not in this repository"
            ],
            problems,
        )


GUIDES = "urn:iki:harness:starter:guide:"


class GuideTextTest(unittest.TestCase):
    """Each guide quotes its section, and a quote is held to the section as a reader sees it,
    in the file the guide's statedIn names, inside this repository."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.outside = Path(self.tmp.name)
        self.repo = self.outside / "repo"
        shutil.copytree(ROOT, self.repo, ignore=shutil.ignore_patterns(".git", "target"))
        self.assertEqual([], harness.check(self.repo), "the copy agrees before it is changed")
        make_only_windows_names(self)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def edit(self, name: str, pattern: str, new: str) -> None:
        path = self.repo / name
        text, n = re.subn(pattern, lambda _: new, path.read_text(encoding="utf-8"), flags=re.M)
        self.assertEqual(1, n, f"{pattern!r} in {name}")
        path.write_text(text, encoding="utf-8")

    def guide(self, guide: str, comment: str | None = None, stated_in: str | None = None) -> None:
        """Give one guide in the copy's harness.ttl a new rdfs:comment or statedIn."""
        path = self.repo / ".gonk/harness/harness.ttl"
        text = path.read_text(encoding="utf-8")
        start = text.index(f"guide:{guide} a harness:Guide ;\n")
        end = text.index(" .\n", start)
        block = text[start:end]
        if comment is not None:
            block, n = re.subn(r'rdfs:comment "[^"\n]*"', f'rdfs:comment "{comment}"', block)
            self.assertEqual(1, n)
        if stated_in is not None:
            self.assertIn('harness:statedIn "AGENTS.md"', block)
            literal = harness.L(stated_in).nt()
            block = block.replace('harness:statedIn "AGENTS.md"', f"harness:statedIn {literal}")
        path.write_text(text[:start] + block + text[end:], encoding="utf-8")

    def restore(self) -> None:
        shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")

    @staticmethod
    def comments() -> dict[str, str]:
        text = (ROOT / ".gonk/harness/harness.ttl").read_text(encoding="utf-8")
        pattern = r'^guide:(\S+) a harness:Guide ;\n.*\n\s+rdfs:comment "([^"\n]*)"'
        return dict(re.findall(pattern, text, re.M))

    def problems(self, guide: str) -> list[str]:
        return [p for p in harness.check(self.repo) if p.startswith(GUIDES + guide + ":")]

    # A guide that quotes nothing of its section is not compared with it, so it is refused.

    def test_a_guide_whose_text_drifts_from_its_section_fails(self) -> None:
        # p2 of ikigai-slack#186: the guide now says the opposite of its section.
        self.guide("ask-dax-dont-guess", "pick the likeliest option and carry on without asking.")
        self.assertEqual(
            [
                f"{GUIDES}ask-dax-dont-guess: quotes nothing of AGENTS.md "
                "`## Ask Dax, don't guess`; quote, in backticks, a phrase the section says"
            ],
            self.problems("ask-dax-dont-guess"),
        )

    def test_a_guide_that_loses_its_only_quote_fails(self) -> None:
        self.guide("gates-before-push", "Run the checks locally before pushing (just gates).")
        self.assertEqual(
            [
                f"{GUIDES}gates-before-push: quotes nothing of AGENTS.md "
                "`## 4. Every change through a pull request with green CI`; "
                "quote, in backticks, a phrase the section says"
            ],
            self.problems("gates-before-push"),
        )

    def test_a_section_that_drops_a_quoted_rule_fails(self) -> None:
        # p2b of ikigai-slack#186: AGENTS.md's copy of a rule says the opposite.
        self.edit(
            "AGENTS.md",
            r"Never pipe a command whose result you are trusting\.",
            "Piping a command whose result you trust is fine.",
        )
        self.assertEqual(1, len(self.problems("last-command-exit-status")))

    def test_a_guide_that_quotes_only_its_heading_fails(self) -> None:
        # ikigai-slack#196: the heading's words alone let the rule's body drift unchecked.
        self.guide("reproduce-before-fix", "`Reproduce before you fix`: do whatever the item says.")
        self.edit(
            "AGENTS.md",
            r"A bug report, a review finding, or a measurement someone else made is a \*\*claim\*\*",
            "Just do what the item says. A bug report is a task",
        )
        self.assertEqual(
            [
                f"{GUIDES}reproduce-before-fix: quotes only the heading of AGENTS.md "
                "`## 3. Reproduce before you fix`; quote, in backticks, a phrase the section "
                "says under it"
            ],
            harness.check(self.repo),
        )

    def test_every_guide_quotes_its_section(self) -> None:
        unquoted = [g for g, c in self.comments().items() if not re.search(r"`[^`\n]+`", c)]
        self.assertEqual([], unquoted)

    # A quote must be in the section the guide cites, not merely somewhere in the file.

    def test_a_quote_from_another_section_fails(self) -> None:
        # `just gates` is in AGENTS.md, in section 4: m1/M3 and M5 of ikigai-slack#190.
        self.guide("no-bare-numbers", "Write `PR #N` for a pull request, and run `just gates`.")
        self.assertEqual(
            [
                f"{GUIDES}no-bare-numbers: quotes `just gates`, which AGENTS.md "
                "`` ## 6. Never a bare `#N` `` does not"
            ],
            self.problems("no-bare-numbers"),
        )

    # The section as a reader sees it.

    def test_a_code_span_wrapped_across_lines_still_matches(self) -> None:
        # P2 of ikigai-slack#188: CommonMark reads the line ending as a space.
        self.edit("AGENTS.md", r"`Supersedes PR #N`\.$", "`Supersedes PR\n   #N`.")
        self.assertEqual([], harness.check(self.repo))

    def test_the_headings_words_are_in_its_section(self) -> None:
        # P3 of ikigai-slack#188.
        self.guide("no-bare-numbers", "`Never a bare #N`: write `PR #N` for a pull request.")
        self.assertEqual([], harness.check(self.repo))

    def test_a_fenced_line_is_not_a_heading(self) -> None:
        # p8 of ikigai-slack#188.
        self.edit(
            "AGENTS.md",
            r"^(?=1\. Open your pull request as a draft)",
            "```sh\n## not a heading\n```\n\n",
        )
        self.assertEqual([], harness.check(self.repo))

    def test_a_line_opening_with_a_backtick_code_span_opens_no_fence(self) -> None:
        # ikigai-slack#198: CommonMark 4.5, a backtick fence's info string holds no backtick.
        self.edit(
            "AGENTS.md",
            r"^(?=1\. Open your pull request as a draft)",
            "``` `#N` ``` is how a code span holds backticks.\n\n",
        )
        self.assertEqual([], harness.check(self.repo))

    def test_a_fence_closes_only_on_its_own_character_at_least_as_long(self) -> None:
        # ikigai-slack#198: a shorter run, or the other character, leaves the fence open.
        self.edit(
            "AGENTS.md",
            r"^(?=1\. Open your pull request as a draft)",
            "````\n```\n~~~~\n## not a heading\n`````\n\n",
        )
        self.assertEqual([], harness.check(self.repo))

    def test_emphasis_and_links_read_as_their_words(self) -> None:
        self.edit(
            "AGENTS.md",
            r"^- \*\*No claim, no write\.\*\*",
            "- **No claim, no write.** Read [the loop](#the-loop).",
        )
        self.guide("claim-before-write", "`No claim, no write. Read the loop.`")
        self.assertEqual([], harness.check(self.repo))

    def test_inline_markdown_reads_as_its_words(self) -> None:
        # ikigai-slack#199: each form AGENTS.md might use, read as a reader sees it.
        for markdown, words in [
            ("*Really* never.", "Really never."),
            ("_Really_ never.", "Really never."),
            ("***Really*** never.", "Really never."),
            ("__Really__ never.", "Really never."),
            ("**a *b* c** d", "a b c d"),
            ("See [the loop](#the-loop \"t\").", "See the loop."),
            ("See [the loop](#the-loop 't').", "See the loop."),
            ("See [the loop](<#the loop> (t)).", "See the loop."),
            ("See ![the loop](loop.png \"t\").", "See the loop."),
            ("2\\*3 never.", "2*3 never."),
            ("\\*not emphasis\\*", "*not emphasis*"),
            ("\\`not code\\`", "`not code`"),
            ("a\\\nb", "a b"),
            ("&amp; &lt;b&gt; &#42;x&#42; &#x2A; &copy;", "& <b> *x* * ©"),
            ("&nosuch; &amp", "&nosuch; &amp"),
            ("<mailto:a@b.c> <https://x.y/z>", "mailto:a@b.c https://x.y/z"),
            ("`a\\*b` `&amp;` `*c*`", "a\\*b &amp; *c*"),
            ("snake_case_name and 2 * 3 and a*b", "snake_case_name and 2 * 3 and a*b"),
            ("*see `x` here*", "see x here"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))

    def test_emphasis_titles_escapes_and_entities_in_agents_md_match(self) -> None:
        # The four reproductions of ikigai-slack#199, through the check.
        for added, quote in [
            ("*Really* never.", "No claim, no write. Really never."),
            ("_Really_ never.", "No claim, no write. Really never."),
            ('See [the loop](#the-loop "t").', "No claim, no write. See the loop."),
            ("2\\*3 never.", "No claim, no write. 2*3 never."),
            ("&amp; never.", "No claim, no write. & never."),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit(
                    "AGENTS.md",
                    r"^- \*\*No claim, no write\.\*\*",
                    f"- **No claim, no write.** {added}",
                )
                self.guide("claim-before-write", f"`{quote}`")
                self.assertEqual([], harness.check(self.repo))

    def test_an_autolink_is_its_address_read_as_nothing_else(self) -> None:
        # ikigai-slack#202: no emphasis and no backslash escape inside an autolink (CommonMark 6.5).
        for markdown, words in [
            ("<https://x.y/_a_>", "https://x.y/_a_"),
            ("<https://x.y/*a*>", "https://x.y/*a*"),
            ("<https://x/\\_a\\_>", "https://x/\\_a\\_"),
            ("*a <https://x.y/*b> c*", "a https://x.y/*b c"),
            ("<a_b_@c.d>", "a_b_@c.d"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))
        self.edit(
            "AGENTS.md",
            r"^- \*\*No claim, no write\.\*\*",
            "- **No claim, no write.** See <https://x.y/_a_>.",
        )
        self.guide("claim-before-write", "`No claim, no write. See https://x.y/_a_.`")
        self.assertEqual([], harness.check(self.repo))

    def test_a_private_use_character_reads_as_itself(self) -> None:
        # ikigai-slack#203: a plane-15 character neither crashes the check nor reads as code.
        for char in ("\U000f0fff", "\U000f0000"):
            with self.subTest(char=hex(ord(char))):
                self.assertEqual(f"x {char} y c", harness.flat(harness.inline(f"x {char} y `c`")))
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.edit(
                    "AGENTS.md",
                    r"^(?=1\. Open your pull request as a draft)",
                    f"x {char} y\n\n",
                )
                self.assertEqual([], harness.check(self.repo))
                section = harness.read_sections(self.repo / "AGENTS.md")["Ask Dax, don't guess"]
                self.assertIn(f"x {char} y", section.body)

    def test_unequal_delimiter_runs_and_balanced_parentheses_read_as_commonmark(self) -> None:
        # ikigai-slack#204: CommonMark 6.2's delimiter runs, and 6.3's link destinations.
        for markdown, words in [
            ("**Really* never.", "*Really never."),
            ("*Really** never.", "Really* never."),
            ("*foo**bar**baz*", "foobarbaz"),
            ("*foo**bar*", "foo**bar"),
            ("***strong emph***", "strong emph"),
            ("**foo*bar*baz**", "foobarbaz"),
            ("*(*foo*)*", "(foo)"),
            ("foo*bar*", "foobar"),
            ("foo_bar_", "foo_bar_"),
            ("_foo_bar_baz_", "foo_bar_baz"),
            ("a * b * c", "a * b * c"),
            ("*a `*`*", "a *"),
            ("See [the loop](x(y)).", "See the loop."),
            ("[link](foo(and(bar)))", "link"),
            ("[link](foo(and(bar))", "[link](foo(and(bar))"),
            ("[link](foo\\)bar)", "link"),
            ("[link]()", "link"),
            ("[link](<a b> \"t\")", "link"),
            ("[*a*](b)*", "a*"),
            ("[a [b](c) d](e)", "[a b d](e)"),
            ("[a `]` b](c)", "a ] b"),
            ("[a] b", "[a] b"),
            ("*a\n\nb*", "*a b*"),
            ("`a\n\nb`", "`a b`"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))
        for added, quote in [
            ("**Really* never.", "No claim, no write. *Really never."),
            ("See [the loop](x(y)).", "No claim, no write. See the loop."),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit(
                    "AGENTS.md",
                    r"^- \*\*No claim, no write\.\*\*",
                    f"- **No claim, no write.** {added}",
                )
                self.guide("claim-before-write", f"`{quote}`")
                self.assertEqual([], harness.check(self.repo))

    def test_entities_are_the_named_or_numbered_character_or_their_text(self) -> None:
        for markdown, words in [
            ("&ampx; &amp;", "&ampx; &"),
            ("&#0; &#x110000;", "� �"),
            ("&#42;a&#42;", "*a*"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))

    def test_a_quote_spanning_the_heading_and_the_body_says_so(self) -> None:
        # ikigai-slack#205: a quote no reader sees, refused with a sentence true for it.
        self.guide("reproduce-before-fix", "`Reproduce before you fix A bug report`")
        self.assertEqual(
            [
                f"{GUIDES}reproduce-before-fix: quotes `Reproduce before you fix A bug report`, "
                "which runs from the heading of AGENTS.md `## 3. Reproduce before you fix` "
                "into its body; quote, in backticks, a phrase of one or the other"
            ],
            harness.check(self.repo),
        )

    def test_a_form_the_check_does_not_read_is_named_when_a_quote_is_not_found(self) -> None:
        # Raw HTML, a link reference definition and a setext underline are not read, so a quote
        # the check can't find there is not called one the section does not say.
        for added, clause in [
            ("<b>Really</b> never.", "the section has raw HTML"),
            ("[Really][r] never.\n\n[r]: https://x.y", "the file has a link reference definition"),
            ("Really\n===\nnever.", "the section has a line of only `=` or `-` under a line of text"),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                self.guide("ask-dax-dont-guess", "`Really never.` `Never mark it ready for review.`")
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-dont-guess: quotes `Really never.`, which the check "
                        f"can't find in AGENTS.md `## Ask Dax, don't guess`: {clause}, "
                        "which the check does not read"
                    ],
                    self.problems("ask-dax-dont-guess"),
                )

    def test_an_image_may_hold_a_link(self) -> None:
        # ikigai-slack#217: a link deactivates only the `[` openers before it, not `![`
        # (CommonMark 6.3), so an image's text may hold a link.
        for markdown, words in [
            ("![a [b](c)](d)", "a b"),
            ("![[[foo](uri1)](uri2)](uri3)", "[foo](uri2)"),
            ("[a ![b](c) d](e)", "a b d"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))
        self.edit(
            "AGENTS.md",
            r"^- \*\*No claim, no write\.\*\*",
            "- **No claim, no write.** See ![a [b](c)](d).",
        )
        self.guide("claim-before-write", "`No claim, no write. See a b.`")
        self.assertEqual([], harness.check(self.repo))

    def test_a_block_form_the_check_does_not_read_is_named(self) -> None:
        # ikigai-slack#218: an indented code block or a block quote is named, not misread.
        for added, quote, clause in [
            ("    *Really* never.", "*Really* never.", "the section has an indented code block"),
            ("\t*Really* never.", "*Really* never.", "the section has an indented code block"),
            ("> Really\n> never.", "Really never.", "the section has a block quote"),
            ("- a\n\n      *Really* never.", "*Really* never.", "the section has an indented code block"),
            ("-     *Really* never.", "*Really* never.", "the section has an indented code block"),
            ("- > Really\n  > never.", "Really never.", "the section has a block quote"),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                self.guide("ask-dax-dont-guess", f"`{quote}` `Never mark it ready for review.`")
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-dont-guess: quotes `{quote}`, which the check "
                        f"can't find in AGENTS.md `## Ask Dax, don't guess`: {clause}, "
                        "which the check does not read"
                    ],
                    self.problems("ask-dax-dont-guess"),
                )

    def test_block_forms_the_check_reads_are_read_as_commonmark_does(self) -> None:
        # ikigai-slack#218: every other block form is read, each as a reader sees it.
        for added, words in [
            # A list item ends the paragraph before it; a line indented to an item's text
            # after a blank line is a paragraph of the item, not code; a marker is its text.
            ("- *Really\n- never.*", "- *Really - never.*"),
            ("- a\n\n  *Really* never.", "- a Really never."),
            ("1. a\n2. *Really\n3. never.*", "1. a 2. *Really 3. never.*"),
            # A list starting at 2 does not interrupt a paragraph, so its line continues it.
            ("*Really\n2. never.*", "Really 2. never."),
            # A thematic break ends a paragraph and shows no words.
            ("*Really\n***\nnever.*", "*Really never.*"),
            ("*Really\n- - -\nnever.*", "*Really never.*"),
            # An ATX heading in a section is its words, closing #s dropped, a block of its own.
            ("### *Really ###\nnever.*", "*Really never.*"),
            ("- ## *Really*\n\nnever.", "- Really never."),
            # A fence in a list item closes at the item's indentation.
            ("- ```\n  *Really*\n  ```\n\nnever.", "- *Really* never."),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                section = harness.read_sections(self.repo / "AGENTS.md")["Ask Dax, don't guess"]
                self.assertIn(f"this file settles. {words} 1. Open", section.body)
                self.assertEqual((), section.unread)

    def test_a_heading_inside_another_block_opens_no_section(self) -> None:
        # ikigai-slack#218: `## ` opens a section only as a top-level ATX heading, and an
        # HTML block runs to its end, a `## ` line inside it included (CommonMark 4.6).
        for added in ["<div>\n## Really\n</div>", "<!--\n\n## Really\n\n-->", "- a\n\n  ## Really"]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                self.assertEqual([], harness.check(self.repo))
                self.assertNotIn("Really", harness.read_sections(self.repo / "AGENTS.md"))
        # A top-level one does, however it is written: indented, by a tab, or closed by #s.
        for heading in [" ## Really ##", "##\tReally", "## Really #"]:
            with self.subTest(heading=heading):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{heading}\n\n")
                self.assertIn("Really", harness.read_sections(self.repo / "AGENTS.md"))

    # statedIn is a file inside this repository, compared by what it resolves to.

    def test_a_stated_in_path_outside_the_repository_fails(self) -> None:
        # P1 and P1b of ikigai-slack#187.
        (self.outside / "x").mkdir()
        shutil.copy(self.repo / "AGENTS.md", self.outside / "x" / "RULES.md")
        for name in ("../x/RULES.md", "out"):
            with self.subTest(name=name):
                if name == "out":
                    symlink_or_skip(self, self.repo / "out", self.outside / "x" / "RULES.md")
                self.restore()
                self.guide("ask-dax-continue-draft", stated_in=name)
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-continue-draft: statedIn {name!r}, "
                        "which is outside this repository"
                    ],
                    self.problems("ask-dax-continue-draft"),
                )

    def test_a_refused_symlink_skips_only_its_own_case(self) -> None:
        # ikigai-slack#241: Windows without Developer Mode or administrator rights refuses a
        # symlink with WinError 1314, and the case that needs one errored. Nothing here runs on
        # Windows, so model the refusal: that case skips with its reason, and the rest still run.
        def refuse(link: Path, target, target_is_directory: bool = False) -> None:
            error = OSError(errno.EINVAL, "A required privilege is not held by the client",
                            os.fspath(target), 1314, os.fspath(link))
            if not hasattr(error, "winerror"):  # only Windows keeps the fourth argument
                error.winerror = 1314
            raise error

        case = GuideTextTest("test_a_stated_in_path_outside_the_repository_fails")
        result = unittest.TestResult()
        with mock.patch.object(Path, "symlink_to", refuse):
            case.run(result)
        self.assertEqual(([], []), (result.errors, result.failures))
        self.assertEqual(1, len(result.skipped), result.skipped)
        skipped, reason = result.skipped[0]
        self.assertIn("name='out'", skipped.id())
        self.assertIn("refuses a symlink", reason)
        self.assertIn("1314", reason)

    def test_a_symlink_error_other_than_1314_is_raised_not_skipped(self) -> None:
        # ikigai-slack#251: symlink_or_skip skips only WinError 1314. Any other OSError, such as
        # a PermissionError with no winerror or a Windows error other than 1314, is a fault the
        # case reports as an error, never as a skip.
        def permission(link: Path, target, target_is_directory: bool = False) -> None:
            raise PermissionError(errno.EPERM, "Operation not permitted", os.fspath(link))

        def access_denied(link: Path, target, target_is_directory: bool = False) -> None:
            error = OSError(errno.EACCES, "Access is denied", os.fspath(target), 5, os.fspath(link))
            if not hasattr(error, "winerror"):  # only Windows keeps the fourth argument
                error.winerror = 5
            raise error

        for refuse in (permission, access_denied):
            with self.subTest(refuse=refuse.__name__):
                case = GuideTextTest("test_a_stated_in_path_outside_the_repository_fails")
                result = unittest.TestResult()
                with mock.patch.object(Path, "symlink_to", refuse):
                    case.run(result)
                self.assertEqual(([], []), (result.skipped, result.failures))
                self.assertEqual(1, len(result.errors), result.errors)
                errored, trace = result.errors[0]
                self.assertIn("name='out'", errored.id())
                self.assertIn(
                    "Operation not permitted" if refuse is permission else "Access is denied",
                    trace,
                )

    def test_an_absolute_stated_in_path_is_not_relative_to_the_repository(self) -> None:
        # ikigai-slack#197: refused whether it names a file outside the repository or inside.
        (self.outside / "x").mkdir()
        shutil.copy(self.repo / "AGENTS.md", self.outside / "x" / "RULES.md")
        # ikigai-slack#235: a drive letter, a URL or any colon is not relative either, on every
        # platform, and a path is written with / so it is one on every platform. A colon is
        # refused from the string alone, so `x:` is never made: Windows can't (ikigai-slack#237).
        for name in (
            *absolute_paths(self.outside, self.repo / "AGENTS.md", "RULES.md"),
            "C:/Windows/win.ini",
            "C:\\x\\RULES.md",
            "C:AGENTS.md",
            "\\\\server\\share\\RULES.md",
            "../x:/RULES.md",
            "file:///etc/passwd",
        ):
            with self.subTest(name=name):
                self.restore()
                self.guide("ask-dax-continue-draft", stated_in=name)
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-continue-draft: statedIn {name!r}, "
                        "which is not a path relative to this repository's root"
                    ],
                    self.problems("ask-dax-continue-draft"),
                )

    def test_a_stated_in_path_with_a_nul_is_not_a_path(self) -> None:
        # Finding 3 of ikigai-slack#235: a sentence, not a ValueError from resolve().
        self.guide("ask-dax-continue-draft", stated_in="a\x00b")
        self.assertEqual(
            [f"{GUIDES}ask-dax-continue-draft: statedIn 'a\\x00b', which is not a path"],
            self.problems("ask-dax-continue-draft"),
        )

    def test_another_spelling_of_agents_md_is_agents_md(self) -> None:
        # P8 of ikigai-slack#187: the guide still cites `## 7. …` of AGENTS.md.
        self.guide("read-accessibility-first", stated_in="./AGENTS.md")
        self.assertEqual([], harness.check(self.repo))

    def test_a_stated_in_file_that_is_not_utf8_is_a_sentence(self) -> None:
        # p6 and P6 of ikigai-slack#189.
        for body in (b"\xff\xfe\n", b"caf\xe9\n"):
            with self.subTest(body=body):
                self.restore()
                (self.repo / "BIN.md").write_bytes(b"## Ask Dax, don't guess\n" + body)
                self.guide("ask-dax-continue-draft", stated_in="BIN.md")
                self.assertEqual(
                    [f"{GUIDES}ask-dax-continue-draft: statedIn 'BIN.md', which is not UTF-8 text"],
                    self.problems("ask-dax-continue-draft"),
                )
                out = io.StringIO()
                with contextlib.redirect_stdout(out):
                    self.assertEqual(1, harness.main(["check", "--repo", str(self.repo)]))
                self.assertIn("which is not UTF-8 text", out.getvalue())


CONTROLS = "urn:iki:harness:starter:control:"


class ControlCodeTest(unittest.TestCase):
    """A control's enforcedAt names a file inside this repository, compared by what it
    resolves to, as a guide's statedIn is (ikigai-slack#193)."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.outside = Path(self.tmp.name)
        self.repo = self.outside / "repo"
        shutil.copytree(ROOT, self.repo, ignore=shutil.ignore_patterns(".git", "target"))
        self.assertEqual([], harness.check(self.repo), "the copy agrees before it is changed")
        make_only_windows_names(self)
        (self.outside / "x").mkdir()
        shutil.copy(self.repo / ".github/workflows/ci.yml", self.outside / "x" / "ci.yml")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def enforced_at(self, where: str) -> list[str]:
        """Point the copy's ctl:ci-checks at `where`, and check: the problems it is named in."""
        path = self.repo / ".gonk/harness/harness.ttl"
        text = path.read_text(encoding="utf-8")
        start = text.index("ctl:ci-checks a harness:Control ;\n")
        end = text.index(" .\n", start)
        block = text[start:end]
        self.assertIn('harness:enforcedAt ".github/workflows/ci.yml"', block)
        literal = harness.L(where).nt()
        block = block.replace('harness:enforcedAt ".github/workflows/ci.yml"', f"harness:enforcedAt {literal}")
        path.write_text(text[:start] + block + text[end:], encoding="utf-8")
        return [p for p in harness.check(self.repo) if p.startswith(CONTROLS + "ci-checks:")]

    def test_an_enforced_at_path_outside_the_repository_fails(self) -> None:
        where = "../x/ci.yml"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} is outside this repository"],
            self.enforced_at(where),
        )

    def test_an_absolute_enforced_at_path_is_not_relative_to_the_repository(self) -> None:
        # Refused whether it names a file outside the repository or inside, as statedIn is, and
        # however it is spelled: a drive letter, a URL or any colon that is not `owner/repo:`
        # (ikigai-slack#235). A path is written with / so it is one on every platform. A colon is
        # refused from the string alone, so `x:` is never made: Windows can't (ikigai-slack#237).
        for where in (
            *absolute_paths(self.outside, self.repo / ".github/workflows/ci.yml", "ci.yml"),
            "C:/Windows/win.ini",
            "C:\\x\\ci.yml",
            "C:.github/workflows/ci.yml",
            "\\\\server\\share\\ci.yml",
            "../x:/ci.yml",
            "../a:b/ci.yml",
            "file:///etc/passwd",
        ):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual(
                    [
                        f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} "
                        "is not a path relative to this repository's root"
                    ],
                    self.enforced_at(where),
                )

    def test_an_enforced_at_path_with_a_nul_is_not_a_path(self) -> None:
        # Finding 3 of ikigai-slack#235: a sentence, not a ValueError from resolve().
        where = "a\x00b"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} is not a path"],
            self.enforced_at(where),
        )

    def test_an_enforced_at_in_another_repository_is_not_read_here(self) -> None:
        # `owner/repo:path`, as vocabulary.ttl gives it, is the only form not read as a path here.
        for where in ("carducci/ROC-slack-door:nowhere.yml", "ikigai-rs/ikigai-ledger:src/x.rs#f"):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual([], self.enforced_at(where))

    def test_an_enforced_at_file_that_is_not_here_still_fails(self) -> None:
        where = ".github/workflows/nowhere.yml"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} is not in this repository"],
            self.enforced_at(where),
        )


if __name__ == "__main__":
    unittest.main()
