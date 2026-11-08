"""`tools/harness.py check`: every rule AGENTS.md states is named by a guide in harness.ttl,
and a guide and the AGENTS.md section it cites agree on the words they quote."""

from __future__ import annotations

import ast
import builtins
import contextlib
import errno
import importlib.util
import io
import os
import re
import shutil
import socket
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


class CodeEncodingTest(unittest.TestCase):
    """A file under tools/ is read as Python reads it: its coding declaration decides, UTF-8 when
    it has none. Only a file its declared encoding can't decode is refused (ikigai-slack#305)."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name) / "repo"
        shutil.copytree(ROOT, self.repo, ignore=shutil.ignore_patterns(".git", "target"))
        self.assertEqual([], harness.check(self.repo), "the copy agrees before it is changed")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def problems_for(self, data: bytes) -> list[str]:
        (self.repo / "tools/m000.py").write_bytes(data)
        return [p for p in harness.emissions(self.repo)[1] if "m000.py" in p]

    def test_a_latin1_cookie_with_latin1_bytes_is_text(self) -> None:
        # ikigai-slack#305: Python compiles this file, so the check must not refuse it.
        data = b'# -*- coding: latin-1 -*-\nx = "caf\xe9"\n'
        self.assertEqual([], self.problems_for(data))
        self.assertEqual([], [p for p in harness.check(self.repo) if "m000.py" in p])

    def test_a_bom_with_a_utf8_cookie_is_text(self) -> None:
        self.assertEqual([], self.problems_for(b'\xef\xbb\xbf# -*- coding: utf-8 -*-\nx = "caf\xc3\xa9"\n'))

    def refused(self, data: bytes) -> str:
        found = self.problems_for(data)
        self.assertEqual(1, len(found), found)
        # What Python says of these same bytes is computed here, so no version's wording is pinned.
        with self.assertRaises((SyntaxError, ValueError)) as said:
            ast.parse(data, "tools/m000.py")
        self.assertEqual(f"tools/m000.py can't be parsed ({said.exception})", found[0])
        return found[0]

    def test_a_utf8_file_with_latin1_bytes_is_refused_with_what_python_said(self) -> None:
        self.refused(b'x = "caf\xe9"\n')

    def test_an_unknown_declared_encoding_is_refused(self) -> None:
        self.assertIn("no-such-codec", self.refused(b"# -*- coding: no-such-codec -*-\nx = 1\n"))

    def test_a_non_text_codec_is_refused_not_a_traceback(self) -> None:
        for codec in ("rot13", "hex", "base64"):
            with self.subTest(codec=codec):
                self.assertIn(codec, self.refused(f"# coding: {codec}\nx = 1\n".encode()))

    def test_a_cookie_decoded_file_that_does_not_parse_is_refused(self) -> None:
        self.refused(b"# coding: latin-1\ndef (\xe9:\n")

    def test_a_utf8_file_that_does_not_parse_is_refused(self) -> None:
        self.refused(b"def (:\n")

    def test_a_nul_in_valid_utf8_is_refused_with_what_python_said(self) -> None:
        self.refused(b"x = 1\x00\n")

    def test_a_bom_with_a_latin1_cookie_is_refused_with_what_python_said(self) -> None:
        self.refused(b"\xef\xbb\xbf# -*- coding: latin-1 -*-\nx = 1\n")

    def test_one_bad_file_does_not_hide_another(self) -> None:
        (self.repo / "tools/m001.py").write_bytes(b"x = '\xe9'\n")
        (self.repo / "tools/m000.py").write_bytes(b"# coding: rot13\nx = 1\n")
        bad = harness.emissions(self.repo)[1]
        self.assertEqual(2, len([p for p in bad if "m00" in p]), bad)


class TurtleEncodingTest(unittest.TestCase):
    """A Turtle file is UTF-8, and a leading byte order mark is not a term of it (ikigai-slack#304)."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name) / "repo"
        shutil.copytree(ROOT, self.repo, ignore=shutil.ignore_patterns(".git", "target"))
        self.assertEqual([], harness.check(self.repo), "the copy agrees before it is changed")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def with_bom(self, name: str) -> None:
        path = self.repo / ".gonk" / "harness" / name
        path.write_bytes(b"\xef\xbb\xbf" + path.read_bytes())

    def test_a_bom_on_harness_ttl_is_not_refused(self) -> None:
        self.with_bom("harness.ttl")
        self.assertEqual([], harness.check(self.repo))

    def test_a_bom_on_vocabulary_ttl_is_not_refused(self) -> None:
        self.with_bom("vocabulary.ttl")
        self.assertEqual([], harness.check(self.repo))

    def test_a_bom_reads_the_same_triples(self) -> None:
        path = self.repo / ".gonk" / "harness" / "harness.ttl"
        plain = harness.read_graph(path).triples
        self.with_bom("harness.ttl")
        self.assertEqual(plain, harness.read_graph(path).triples)


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

    # A quote is a substring of the cited section's raw Markdown source (ikigai-slack#222).

    def test_a_quote_is_a_substring_of_the_sections_raw_source(self) -> None:
        self.edit(
            "AGENTS.md",
            r"^- \*\*No claim, no write\.\*\*",
            "- **No claim, no write.** Read [the loop](#the-loop).",
        )
        self.guide("claim-before-write", "`**No claim, no write.** Read [the loop](#the-loop).`")
        self.assertEqual([], harness.check(self.repo))
        # What a reader sees is not the source, so it is not a quote of it.
        self.guide("claim-before-write", "`No claim, no write. Read the loop.`")
        self.assertEqual(1, len(self.problems("claim-before-write")))
        # Whitespace runs are collapsed, on both sides.
        self.guide("claim-before-write", "`**No claim, no write.**   Read [the loop](#the-loop).`")
        self.assertEqual([], harness.check(self.repo))

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

    def test_a_quote_is_not_read_through_markdown(self) -> None:
        # Raw HTML, a setext underline and a reference definition are source like any other.
        for added, quote in [
            ("<b>Really</b> never.", "<b>Really</b> never."),
            ("[Really][r] never.\n\n[r]: https://x.y", "[Really][r] never."),
            ("*Really* never.", "*Really* never."),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                self.guide("ask-dax-dont-guess", f"`{quote}` `Never mark it ready for review.`")
                self.assertEqual([], harness.check(self.repo))

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

    def test_a_stated_in_path_that_comes_back_by_the_checkout_name_fails(self) -> None:
        # The copy is named `repo`, so `../repo/AGENTS.md` resolves back inside it. The answer must
        # not depend on the checkout's directory name (ledger item 233).
        name = f"../{self.repo.name}/AGENTS.md"
        self.guide("ask-dax-continue-draft", stated_in=name)
        self.assertEqual(
            [f"{GUIDES}ask-dax-continue-draft: statedIn {name!r}, which is outside this repository"],
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

    def test_a_stated_in_directory_is_a_directory_not_a_file(self) -> None:
        # ikigai-slack#234: `tools` is in the repository; what is wrong is that it is no file.
        for name in ("tools", "tools/", "./tools"):
            with self.subTest(name=name):
                self.restore()
                self.guide("ask-dax-continue-draft", stated_in=name)
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-continue-draft: statedIn {name!r}, "
                        "which is a directory, not a file"
                    ],
                    self.problems("ask-dax-continue-draft"),
                )

    def test_an_empty_stated_in_is_empty(self) -> None:
        # ikigai-slack#234: an empty name resolves to the root, which is in the repository.
        self.guide("ask-dax-continue-draft", stated_in="")
        self.assertEqual(
            [f"{GUIDES}ask-dax-continue-draft: statedIn '', which is empty"],
            self.problems("ask-dax-continue-draft"),
        )

    @unittest.skipUnless(hasattr(os, "mkfifo"), "a FIFO is made by os.mkfifo, POSIX only")
    def test_a_stated_in_that_is_neither_file_nor_directory_is_not_a_file(self) -> None:
        # ikigai-slack#234: a FIFO is in the repository too, and is not a file. Only repo_file
        # is asked: `check` reads what repo_file passes, and reading a FIFO nothing writes to
        # blocks forever, so if the branch regressed, the check would hang instead of failing
        # (ikigai-slack#256). The socket case runs the check through.
        os.mkfifo(self.repo / "PIPE.md")
        self.assertEqual((None, "is not a file"), harness.repo_file(self.repo.resolve(), "PIPE.md"))

    @unittest.skipUnless(hasattr(socket, "AF_UNIX"), "a socket file needs AF_UNIX, POSIX only")
    def test_a_stated_in_socket_is_not_a_file(self) -> None:
        # ikigai-slack#234 and ikigai-slack#256: a socket is in the repository and is no file,
        # and opening one to read it fails at once (ENXIO), so if repo_file let it through this
        # would fail with the system's words, never hang.
        with socket.socket(socket.AF_UNIX) as sock:
            sock.bind(os.fspath(self.repo / "SOCK.md"))
        self.guide("ask-dax-continue-draft", stated_in="SOCK.md")
        self.assertEqual(
            [f"{GUIDES}ask-dax-continue-draft: statedIn 'SOCK.md', which is not a file"],
            self.problems("ask-dax-continue-draft"),
        )

    def test_a_stated_in_symlink_that_resolves_to_nothing_is_said_so(self) -> None:
        # ikigai-slack#257: a symlink to a file that is not there, or one that loops, is in the
        # repository; what is wrong is where it points. On Python 3.12 a loop made resolve()
        # raise RuntimeError, so the check crashed rather than say either.
        for name, links, why in (
            ("dangling.md", [("dangling.md", "missing.md")], "is a symlink to nothing"),
            ("loop", [("loop", "loop")], "is a symlink loop"),
            ("ping.md", [("ping.md", "pong.md"), ("pong.md", "ping.md")], "is a symlink loop"),
        ):
            with self.subTest(name=name):
                for link, target in links:
                    symlink_or_skip(self, self.repo / link, Path(target))
                self.restore()
                self.guide("ask-dax-continue-draft", stated_in=name)
                self.assertEqual(
                    [f"{GUIDES}ask-dax-continue-draft: statedIn {name!r}, which {why}"],
                    self.problems("ask-dax-continue-draft"),
                )

    @unittest.skipIf(os.name == "nt", "a mode of 000 stops nothing on Windows")
    @unittest.skipIf(getattr(os, "geteuid", lambda: 1)() == 0, "root reads a mode-000 directory")
    def test_a_stated_in_under_a_directory_that_cannot_be_searched_cannot_be_read(self) -> None:
        # ikigai-slack#257: in the repository, and refused with the system's reason, as a file
        # that can't be read is. On Python 3.12 exists() raised the PermissionError instead.
        locked = self.repo / "locked"
        locked.mkdir()
        shutil.copy(self.repo / "AGENTS.md", locked / "AGENTS.md")
        locked.chmod(0)
        try:
            self.guide("ask-dax-continue-draft", stated_in="locked/AGENTS.md")
            self.assertEqual(
                [
                    f"{GUIDES}ask-dax-continue-draft: statedIn 'locked/AGENTS.md', "
                    f"which can't be read ({os.strerror(errno.EACCES)})"
                ],
                self.problems("ask-dax-continue-draft"),
            )
        finally:
            locked.chmod(0o755)

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
    """A control's enforcedAt names a file inside this repository, as a guide's statedIn does
    (ikigai-slack#193). Any `..` segment, by `/` or `\\`, is refused as written, before anything
    is resolved, with the reason "has a `..` segment" (or "is outside this repository" when it
    leaves the root); otherwise it is compared by what it resolves to, so a symlink is still
    judged by its resolved target."""

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

    def test_an_enforced_at_path_that_comes_back_by_the_checkout_name_fails(self) -> None:
        # The copy is named `repo`, so `../repo/` resolves back inside it. The answer must not
        # depend on the checkout's directory name: refused, as it is in a checkout named `other`.
        where = f"../{self.repo.name}/.github/workflows/ci.yml"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} is outside this repository"],
            self.enforced_at(where),
        )

    def test_an_enforced_at_path_with_an_inner_dot_dot_segment_fails(self) -> None:
        # Inside the repository once it is resolved, but not a path in its normalised form.
        where = ".github/../.github/workflows/ci.yml"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} has a `..` segment"],
            self.enforced_at(where),
        )

    def test_a_backslash_dot_dot_segment_is_refused_for_its_own_reason_on_every_platform(self) -> None:
        # On Windows `\` separates; on Linux `..\repo\x` is one file name. The reason tells a
        # split on `[\\/]` from a split on `/` alone, which "is not in this repository" would not.
        for where, reason in (
            (f"..\\{self.repo.name}\\.github\\workflows\\ci.yml", "is outside this repository"),
            ("tools\\..\\AGENTS.md", "has a `..` segment"),
        ):
            with self.subTest(where=where):
                self.assertEqual((None, reason), harness.repo_file(self.repo, where))

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

    def test_an_enforced_at_directory_is_a_directory_not_a_file(self) -> None:
        # ikigai-slack#234: `tools` is in the repository; what is wrong is that it is no file.
        # Under tools/ (a code root) the sensor is also asked for a firing test, as it would be
        # for a file there: that problem is its own, and this test only wants it not named as
        # absent.
        for where, path in (
            ("tools", "tools"),
            (".github/workflows", ".github/workflows"),
            (".github/workflows/", ".github/workflows/"),
            (".github/workflows#main", ".github/workflows"),
        ):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                problems = self.enforced_at(where)
                self.assertEqual(
                    f"{CONTROLS}ci-checks: enforcedAt {where}, and {path} is a directory, not a file",
                    problems[0],
                )
                self.assertEqual([], [p for p in problems if "not in this repository" in p])
                if not where.startswith("tools"):
                    self.assertEqual(1, len(problems), "\n".join(problems))

    def test_an_empty_enforced_at_is_empty(self) -> None:
        # ikigai-slack#234: an empty path resolves to the root, which is in the repository. It
        # is shown quoted, as nothing else makes it visible, and `#main` alone has one too.
        for where in ("", "#main"):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual(
                    [f"{CONTROLS}ci-checks: enforcedAt {where!r}, and its path is empty"],
                    self.enforced_at(where),
                )

    @unittest.skipUnless(hasattr(os, "mkfifo"), "a FIFO is made by os.mkfifo, POSIX only")
    def test_an_enforced_at_that_is_neither_file_nor_directory_is_not_a_file(self) -> None:
        # ikigai-slack#234: a FIFO is in the repository too, and is not a file. Only repo_file
        # is asked, as in GuideTextTest: reading a FIFO nothing writes to blocks forever
        # (ikigai-slack#256). The socket case runs the check through.
        os.mkfifo(self.repo / ".github/workflows/pipe.yml")
        self.assertEqual(
            (None, "is not a file"),
            harness.repo_file(self.repo.resolve(), ".github/workflows/pipe.yml"),
        )

    @unittest.skipUnless(hasattr(socket, "AF_UNIX"), "a socket file needs AF_UNIX, POSIX only")
    def test_an_enforced_at_socket_is_not_a_file(self) -> None:
        # ikigai-slack#234 and ikigai-slack#256: a socket is no file, and opening one to read
        # it fails at once (ENXIO), so this fails rather than hangs if repo_file lets it through.
        # `#main` makes the check read the file for the name, as a statedIn is always read.
        with socket.socket(socket.AF_UNIX) as sock:
            sock.bind(os.fspath(self.repo / ".github/workflows/sock.yml"))
        where = ".github/workflows/sock.yml#main"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and .github/workflows/sock.yml is not a file"],
            self.enforced_at(where),
        )

    def test_an_enforced_at_symlink_that_resolves_to_nothing_is_said_so(self) -> None:
        # ikigai-slack#257: a symlink to a file that is not there, or one that loops, is in the
        # repository; what is wrong is where it points. On Python 3.12 a loop made resolve()
        # raise RuntimeError, so the check crashed rather than say either.
        for where, links, why in (
            ("dangling.yml", [("dangling.yml", "missing.yml")], "is a symlink to nothing"),
            ("loop", [("loop", "loop")], "is a symlink loop"),
            ("ping.yml", [("ping.yml", "pong.yml"), ("pong.yml", "ping.yml")], "is a symlink loop"),
        ):
            with self.subTest(where=where):
                for link, target in links:
                    symlink_or_skip(self, self.repo / link, Path(target))
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual(
                    [f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} {why}"],
                    self.enforced_at(where),
                )

    def test_an_enforced_at_with_nothing_after_its_hash_names_no_function(self) -> None:
        # ikigai-slack#261: vocabulary.ttl gives `path#function`; `path#` was taken as `path`.
        for where in (".github/workflows/ci.yml#", "AGENTS.md#", "justfile#"):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual(
                    [f"{CONTROLS}ci-checks: enforcedAt {where}, which names no function after #"],
                    self.enforced_at(where),
                )

    def test_a_name_in_a_file_with_no_known_definitions_is_refused(self) -> None:
        # ikigai-slack#261: in a file neither .py nor a justfile, any substring passed, `x` in
        # AGENTS.md among them, and `#<module>` passed in any file.
        for where, path in (
            ("AGENTS.md#x", "AGENTS.md"),
            (".github/workflows/ci.yml#jobs", ".github/workflows/ci.yml"),
            (".github/workflows/ci.yml#<module>", ".github/workflows/ci.yml"),
        ):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual(
                    [
                        f"{CONTROLS}ci-checks: enforcedAt {where}, and a name can't be checked in "
                        f"{path}: name a function of a .py file or a recipe of a justfile, or "
                        "cite the file alone"
                    ],
                    self.enforced_at(where),
                )

    def test_a_name_only_mentioned_in_a_file_is_not_defined_there(self) -> None:
        # ikigai-slack#261: a .py name is a def or class the code has, not a line in a string
        # that reads like one; a recipe is the whole name, not one a longer recipe starts with.
        (self.repo / ".github/x.py").write_text('"""\ndef ghost():\n"""\n\n\ndef real():\n    pass\n')
        (self.repo / ".github/justfile").write_text("worker-once:\n    echo\n\n@quiet arg='a:b':\n    echo\n")
        for where, ok in (
            (".github/x.py#ghost", False),
            (".github/x.py#real", True),
            (".github/x.py#<module>", True),
            (".github/justfile#worker", False),
            (".github/justfile#worker-once", True),
            (".github/justfile#quiet", True),
        ):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                path, _, name = where.partition("#")
                expected = [] if ok else [f"{CONTROLS}ci-checks: enforcedAt {where}, and {path} has no {name}"]
                self.assertEqual(expected, self.enforced_at(where))

    def test_a_name_that_is_no_recipe_name_or_identifier_is_refused(self) -> None:
        # ikigai-slack#278: a name of spaces matched a body line's indent, and a name with a
        # trailing space or a recipe line's start matched a recipe with parameters.
        (self.repo / ".github/justfile").write_text(
            "real arg=`a:=b`:\n    echo hi: there\n\ninit *flags:\n    echo\n\n# x\nnext:\n    echo\n"
        )
        (self.repo / ".github/x.py").write_text("def real():\n    pass\n")
        just = "is not a recipe name of a justfile ([A-Za-z_][A-Za-z0-9_-]*)"
        py = "is not an identifier of a .py file, or <module>"
        for where, path, name, why in (
            (".github/justfile#   ", ".github/justfile", "   ", just),
            (".github/justfile#init ", ".github/justfile", "init ", just),
            (".github/justfile#    echo hi", ".github/justfile", "    echo hi", just),
            (".github/justfile#init *flags", ".github/justfile", "init *flags", just),
            (".github/justfile## x\nnext", ".github/justfile", "# x\nnext", just),
            (".github/justfile#-real", ".github/justfile", "-real", just),
            (".github/justfile#next\n", ".github/justfile", "next\n", just),
            (".github/x.py#real ", ".github/x.py", "real ", py),
            (".github/x.py#def real", ".github/x.py", "def real", py),
        ):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual(
                    [f"{CONTROLS}ci-checks: enforcedAt {where}, and {path}: {name!r} {why}"],
                    self.enforced_at(where),
                )

    def test_every_file_name_just_reads_is_a_justfile(self) -> None:
        # ikigai-slack#278: just finds `justfile` in any case and `.justfile`, and loads a
        # module from `<name>.just`; a recipe cited in any of them is a recipe.
        for file in ("Justfile", ".justfile", "JUSTFILE", "m.just"):
            (self.repo / ".github" / file).write_text("build:\n    echo\n")
            for name, ok in (("build", True), ("buil", False)):
                where = f".github/{file}#{name}"
                with self.subTest(where=where):
                    shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                    expected = [] if ok else [f"{CONTROLS}ci-checks: enforcedAt {where}, and .github/{file} has no {name}"]
                    self.assertEqual(expected, self.enforced_at(where))

    def test_a_quoted_default_may_hold_a_colon_equals(self) -> None:
        # ikigai-slack#278: only a quoted `:=` needs the quote branch; an unquoted one ends the
        # recipe line's search, as just reads it as an assignment.
        (self.repo / ".github/justfile").write_text(
            "dq x=\"a:=b\":\n    echo\n\nsq y='a:=b':\n    echo\n\nbare := \"x\"\n"
        )
        for where, ok in (
            (".github/justfile#dq", True),
            (".github/justfile#sq", True),
            (".github/justfile#bare", False),
        ):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                path, _, name = where.partition("#")
                expected = [] if ok else [f"{CONTROLS}ci-checks: enforcedAt {where}, and {path} has no {name}"]
                self.assertEqual(expected, self.enforced_at(where))

    def test_a_recipe_looking_line_in_a_triple_quoted_string_is_not_a_recipe(self) -> None:
        # ikigai-slack#273: the body of an indented string is text, whichever quote kind.
        (self.repo / ".github/justfile").write_text(
            "a := '''\nsq: a\n'''\nb := \"\"\"\ndq: in a string\n\"\"\"\n"
            "c := ```\nbt: x\n```\n\nreal:\n    echo\n"
        )
        for where, ok in (
            (".github/justfile#sq", False),
            (".github/justfile#dq", False),
            (".github/justfile#bt", False),
            (".github/justfile#real", True),
        ):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                path, _, name = where.partition("#")
                expected = [] if ok else [f"{CONTROLS}ci-checks: enforcedAt {where}, and {path} has no {name}"]
                self.assertEqual(expected, self.enforced_at(where))

    def assert_recipes(self, justfile: str, expected: dict[str, bool]) -> None:
        (self.repo / ".github/justfile").write_text(justfile)
        for name, ok in expected.items():
            where = f".github/justfile#{name}"
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                want = [] if ok else [f"{CONTROLS}ci-checks: enforcedAt {where}, and .github/justfile has no {name}"]
                self.assertEqual(want, self.enforced_at(where))

    def test_two_triple_quoted_strings_leave_the_recipe_between_them(self) -> None:
        # ikigai-slack#308: a greedy string pattern hides `real`, which sits between two strings.
        self.assert_recipes(
            "a := '''\nsq: x\n'''\n\nreal:\n    echo\n\nb := '''\nsq2: y\n'''\n",
            {"real": True, "sq": False, "sq2": False},
        )

    def test_a_quote_triple_outside_a_string_opens_none(self) -> None:
        # ikigai-slack#307: in a comment, a one-line string, a default or a recipe body.
        self.assert_recipes(
            "# a ''' in a comment\nfirst:\n    echo\n\none := \"a'''b\"\n\nreal1:\n    echo\n\n"
            "dflt x='\"\"\"':\n    echo\n\nbody:\n    echo '''\n\nreal2:\n    echo\n",
            {"first": True, "real1": True, "dflt": True, "body": True, "real2": True},
        )

    def test_a_ghost_recipe_in_a_string_after_a_quote_triple_is_not_a_recipe(self) -> None:
        # ikigai-slack#307: a `'''` in a comment or a one-line string before the real string
        # shifted the pairing; and `\\"""` is an escape, not a close.
        self.assert_recipes(
            "# '''\nx := '''\nghost1: a\n'''\n\ny := \"it'''s\" + \"\"\"\n  ghost2: b\n\"\"\"\n\n"
            'z := """\n\\"""\nghost3: c\n"""\n\nreal:\n    echo\n',
            {"ghost1": False, "ghost2": False, "ghost3": False, "real": True},
        )

    def test_a_backtick_default_and_an_escaped_quote_may_hold_a_colon_equals(self) -> None:
        # ikigai-slack#275: a backtick default is a quote kind too, and a double-quoted default
        # may hold an escaped quote before its `:=`.
        (self.repo / ".github/justfile").write_text(
            'real arg=`a:=b`:\n    echo\n\nesc x="a\\"b:=c":\n    echo\n\nbt y=`a:b`:\n    echo\n'
        )
        for where in (".github/justfile#real", ".github/justfile#esc", ".github/justfile#bt"):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual([], self.enforced_at(where))

    def test_a_class_and_an_async_def_are_defined(self) -> None:
        (self.repo / ".github/x.py").write_text("class Thing:\n    pass\n\n\nasync def wait():\n    pass\n")
        for where in (".github/x.py#Thing", ".github/x.py#wait"):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual([], self.enforced_at(where))

    def test_a_py_file_that_cannot_be_parsed_is_said_so(self) -> None:
        # ikigai-slack#278: a SyntaxError, and a source too deep for the parser, which raises
        # MemoryError (or RecursionError), not SyntaxError, and crashed the check.
        for source in ("def a(:\n", "x = " + "-" * 100000 + "1\n\n\ndef a():\n    pass\n"):
            with self.subTest(source=source[:12]):
                (self.repo / ".github/x.py").write_text(source)
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                [problem] = self.enforced_at(".github/x.py#a")
                self.assertTrue(
                    problem.startswith(f"{CONTROLS}ci-checks: enforcedAt .github/x.py#a, and .github/x.py can't be parsed ("),
                    problem,
                )

    @unittest.skipIf(os.name == "nt", "a mode of 000 stops nothing on Windows")
    @unittest.skipIf(getattr(os, "geteuid", lambda: 1)() == 0, "root reads a mode-000 directory")
    def test_an_enforced_at_under_a_directory_that_cannot_be_searched_cannot_be_read(self) -> None:
        # ikigai-slack#257: in the repository, and refused with the system's reason, as a file
        # that can't be read is. On Python 3.12 exists() raised the PermissionError instead.
        locked = self.repo / "locked"
        locked.mkdir()
        shutil.copy(self.repo / ".github/workflows/ci.yml", locked / "ci.yml")
        locked.chmod(0)
        try:
            where = "locked/ci.yml"
            self.assertEqual(
                [
                    f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} "
                    f"can't be read ({os.strerror(errno.EACCES)})"
                ],
                self.enforced_at(where),
            )
        finally:
            locked.chmod(0o755)

    @unittest.skipIf(os.name == "nt", "a mode of 000 stops nothing on Windows")
    @unittest.skipIf(getattr(os, "geteuid", lambda: 1)() == 0, "root reads a mode-000 file")
    def test_an_enforced_at_file_with_a_name_that_cannot_be_read_is_said_so(self) -> None:
        # ikigai-slack#264: repo_file passes a regular file it can't read, and a `#name` makes
        # the check read it for the name; that read raised PermissionError rather than say so.
        # Only a .py file or a justfile is read for a name (ikigai-slack#261), so it is a justfile.
        locked = self.repo / "m000.just"
        locked.write_text("main:\n    true\n", encoding="utf-8")
        locked.chmod(0)
        try:
            where = "m000.just#main"
            self.assertEqual(
                [
                    f"{CONTROLS}ci-checks: enforcedAt {where}, and m000.just "
                    f"can't be read ({os.strerror(errno.EACCES)})"
                ],
                self.enforced_at(where),
            )
        finally:
            locked.chmod(0o644)

    @unittest.skipIf(os.name == "nt", "a mode of 000 stops nothing on Windows")
    @unittest.skipIf(getattr(os, "geteuid", lambda: 1)() == 0, "root reads a mode-000 file")
    def test_an_enforced_at_code_file_that_cannot_be_read_is_said_so(self) -> None:
        # ikigai-slack#264: a file under tools/ is read for its emitters too, and that read
        # raised PermissionError as well. Naming code makes ci-checks a code sensor, so the
        # problems that follow from that come after.
        locked = self.repo / "tools/m000.py"
        locked.write_text("def main():\n    pass\n", encoding="utf-8")
        locked.chmod(0)
        try:
            where = "tools/m000.py#main"
            reason = f"can't be read ({os.strerror(errno.EACCES)})"
            self.assertEqual(
                f"{CONTROLS}ci-checks: enforcedAt {where}, and tools/m000.py {reason}",
                self.enforced_at(where)[0],
            )
            self.assertIn(f"tools/m000.py {reason}", harness.check(self.repo))
        finally:
            locked.chmod(0o644)

    def test_an_enforced_at_file_with_a_name_that_is_not_utf8_is_said_so(self) -> None:
        # ikigai-slack#264: the same read raised UnicodeDecodeError; a statedIn says this.
        (self.repo / "binary.just").write_bytes(b"main:\n    \xff\xfe\n")
        where = "binary.just#main"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and binary.just is not UTF-8 text"],
            self.enforced_at(where),
        )

    def test_a_py_file_with_a_bom_parses(self) -> None:
        # ikigai-slack#282: read_text kept the BOM and ast.parse raised SyntaxError.
        (self.repo / "tools/bom.py").write_bytes(b"\xef\xbb\xbfdef main():\n    pass\n")
        self.assertEqual([], harness.check(self.repo))

    def test_a_py_file_ast_rejects_is_said_so(self) -> None:
        for data, reason in (
            (b"def (:\n", "invalid syntax"),
            (b"x = 1\x00\n", "null bytes"),
        ):
            with self.subTest(data=data):
                (self.repo / "tools/bad.py").write_bytes(data)
                problems = [p for p in harness.check(self.repo) if p.startswith("tools/bad.py can't be parsed (")]
                self.assertEqual(1, len(problems), problems)
                self.assertIn(reason, problems[0])

    def test_a_justfile_with_a_bom_has_its_recipe(self) -> None:
        (self.repo / "bom.just").write_bytes(b"\xef\xbb\xbfmain:\n    echo hi\n")
        self.assertIsNone(harness.undefined_in(self.repo / "bom.just", "bom.just", "main"))


if __name__ == "__main__":
    unittest.main()
