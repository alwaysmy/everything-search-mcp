"""Tests for everything_mcp.server tool functions and helpers."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from everything_mcp.backend import SearchResult
from everything_mcp.server import (
    _TEXT_EXTENSIONS,
    _TEXT_FILENAMES,
    _format_search_results,
    _get_file_details_sync,
    _read_preview,
)

# ── _format_search_results ────────────────────────────────────────────────


class TestFormatSearchResults:
    def test_empty_results(self):
        result = _format_search_results([], "*.py", max_results=50)
        assert "No results found" in result
        assert "*.py" in result

    def test_with_results(self, sample_results):
        result = _format_search_results(sample_results, "*.py", max_results=50)
        assert "Found 3 results" in result
        assert r"C:\Projects\app\main.py" in result
        assert r"C:\Projects\app\utils.py" in result
        assert "[FILE]" in result
        assert "[DIR]" in result

    def test_file_metadata_shown(self, sample_results):
        result = _format_search_results(sample_results, "*.py", max_results=50)
        assert "2.0 KB" in result
        assert "2026-01-15" in result

    def test_offset_shown(self, sample_results):
        result = _format_search_results(sample_results, "*.py", max_results=50, offset=100)
        assert "offset: 100" in result

    def test_pagination_hint_when_at_limit(self, sample_results):
        # When results == max_results, show pagination hint
        result = _format_search_results(sample_results, "*.py", max_results=3)
        assert "offset" in result.lower()

    def test_no_pagination_hint_when_under_limit(self, sample_results):
        result = _format_search_results(sample_results, "*.py", max_results=100)
        assert "Showing first" not in result

    def test_folder_no_size(self, sample_results):
        result = _format_search_results(sample_results, "*", max_results=50)
        # The directory "src" line should not have a size
        lines = result.split("\n")
        src_line = [line for line in lines if "src" in line and "[DIR]" in line]
        assert len(src_line) == 1
        # Should not show "unknown" or negative size
        assert "unknown" not in src_line[0]


# ── _read_preview ─────────────────────────────────────────────────────────


class TestReadPreview:
    def test_reads_python_file(self, tmp_path):
        f = tmp_path / "test.py"
        f.write_text("line 1\nline 2\nline 3\n")
        result = _read_preview(f, 2)
        assert result == "line 1\nline 2"

    def test_reads_all_lines_when_fewer_than_max(self, tmp_path):
        f = tmp_path / "short.py"
        f.write_text("only line\n")
        result = _read_preview(f, 100)
        assert result == "only line"

    def test_returns_none_for_binary(self, tmp_path):
        f = tmp_path / "image.bin"
        f.write_bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00")
        result = _read_preview(f, 10)
        assert result is None

    def test_returns_none_for_unknown_extension(self, tmp_path):
        # Create a binary file with unknown extension
        f = tmp_path / "data.xyz123"
        f.write_bytes(b"\x00\x01\x02\x03")
        result = _read_preview(f, 10)
        assert result is None

    def test_known_filenames(self, tmp_path):
        for name in ("Makefile", "Dockerfile", "LICENSE", "README"):
            f = tmp_path / name
            f.write_text(f"content of {name}\n")
            result = _read_preview(f, 1)
            assert result is not None, f"Failed for {name}"

    def test_dotfile_is_text(self, tmp_path):
        f = tmp_path / ".gitignore"
        f.write_text("node_modules/\n")
        result = _read_preview(f, 1)
        assert result == "node_modules/"

    def test_large_file_returns_message(self, tmp_path):
        f = tmp_path / "huge.txt"
        f.write_bytes(b"x" * (11 * 1024 * 1024))  # 11 MB
        result = _read_preview(f, 10)
        assert result is not None
        assert "too large" in result

    def test_utf8_content(self, tmp_path):
        f = tmp_path / "unicode.py"
        f.write_text("# Hälsningar från Sverige\n", encoding="utf-8")
        result = _read_preview(f, 1)
        assert "Sverige" in result

    def test_latin1_fallback(self, tmp_path):
        f = tmp_path / "latin.txt"
        f.write_bytes("café\n".encode("latin-1"))
        result = _read_preview(f, 1)
        assert result is not None
        assert "caf" in result

    def test_gb18030_fallback(self, tmp_path):
        """Chinese GBK text (common on Chinese Windows) must decode, not mojibake."""
        f = tmp_path / "chinese_gbk.txt"
        f.write_bytes("中文测试文件内容\n".encode("gb18030"))
        result = _read_preview(f, 1)
        assert result is not None
        assert "中文测试文件内容" in result
        assert "\ufffd" not in result  # no replacement chars

    def test_nonexistent_file(self, tmp_path):
        f = tmp_path / "missing.txt"
        result = _read_preview(f, 10)
        assert result is None

    def test_preview_is_char_capped(self, tmp_path):
        f = tmp_path / "long_line.txt"
        f.write_text("x" * 70_000 + "\n")
        result = _read_preview(f, 1)
        assert result is not None
        assert "preview truncated" in result


# ── Extension/filename sets ───────────────────────────────────────────────


class TestTextSets:
    def test_common_code_extensions(self):
        for ext in ("py", "js", "ts", "c", "cpp", "go", "rs", "java"):
            assert ext in _TEXT_EXTENSIONS, f"Missing: {ext}"

    def test_common_config_extensions(self):
        for ext in ("json", "yaml", "yml", "toml", "ini", "xml"):
            assert ext in _TEXT_EXTENSIONS, f"Missing: {ext}"

    def test_common_text_filenames(self):
        for name in ("makefile", "dockerfile", "license", "readme"):
            assert name in _TEXT_FILENAMES, f"Missing: {name}"

    def test_modern_web_extensions(self):
        """Verify modern web framework extensions are supported."""
        for ext in ("astro", "mdx", "svelte", "vue", "prisma"):
            assert ext in _TEXT_EXTENSIONS, f"Missing modern extension: {ext}"


# ── Sort validation ────────────────────────────────────────────────────────


class TestSortValidation:
    """Test that invalid sort options are rejected."""

    def test_valid_sort_accepted(self):
        from everything_mcp.server import SearchInput

        params = SearchInput(query="*.py", sort="date-modified-desc")
        assert params.sort == "date-modified-desc"

    def test_invalid_sort_rejected(self):
        from pydantic import ValidationError

        from everything_mcp.server import SearchInput

        with pytest.raises(ValidationError, match="Invalid sort option"):
            SearchInput(query="*.py", sort="invalid-sort")

    def test_all_sort_options_valid(self):
        from everything_mcp.backend import SORT_MAP
        from everything_mcp.server import SearchInput

        for sort_key in SORT_MAP:
            params = SearchInput(query="test", sort=sort_key)
            assert params.sort == sort_key


# ── Tool error handling ───────────────────────────────────────────────────


class TestToolSuccessPaths:
    def test_file_details_directory_summary(self, tmp_path):
        (tmp_path / "sub").mkdir()
        (tmp_path / "a.txt").write_text("alpha")
        (tmp_path / "b.py").write_text("print('ok')\n")

        result = _get_file_details_sync([str(tmp_path)], preview_lines=0)
        data = json.loads(result)

        assert data["type"] == "folder"
        assert data["item_count"] >= 3
        assert isinstance(data["subdirectories"], list)
        assert isinstance(data["files_sample"], list)

    def test_file_details_rejects_relative_path(self):
        result = _get_file_details_sync(["relative\\file.txt"], preview_lines=0)
        data = json.loads(result)
        assert data["error"] == "Path must be absolute (e.g. C:\\folder\\file)"

    def test_file_details_normalizes_dotdot(self, tmp_path):
        (tmp_path / "a.txt").write_text("alpha")
        # "sub/.." resolves back to tmp_path
        weird = str(tmp_path / "sub" / "..")
        result = _get_file_details_sync([weird], preview_lines=0)
        data = json.loads(result)
        assert data["type"] == "folder"
        assert Path(data["path"]) == tmp_path

    @pytest.mark.asyncio
    async def test_count_stats_breakdown_excludes_directories(self):
        from everything_mcp import server
        from everything_mcp.config import EverythingConfig
        from everything_mcp.server import CountStatsInput, everything_count_stats

        class FakeBackend:
            seen_paths: list[str] = []

            async def count(self, query: str, path_filter: str = "") -> int:
                self.seen_paths.append(path_filter)
                return 3

            async def get_total_size(self, query: str, path_filter: str = "") -> int:
                return 1300

            async def search(self, query: str, max_results: int, sort: str, path_filter: str = ""):
                self.seen_paths.append(path_filter)
                return [
                    SearchResult(path=r"C:\repo\a.py", name="a.py", size=1200, extension="py"),
                    SearchResult(path=r"C:\repo\README", name="README", size=100, extension=""),
                    SearchResult(path=r"C:\repo\src", name="src", is_dir=True),
                ]

        old_backend = server._backend
        old_config = server._config
        try:
            fb = FakeBackend()
            server._backend = fb
            server._config = EverythingConfig(es_path=r"C:\Program Files\Everything\es.exe")

            result = await everything_count_stats(
                CountStatsInput(query="ext:py", breakdown_by_extension=True, path=r"C:\repo")
            )
            data = json.loads(result)

            assert data["total_count"] == 3
            assert data["total_size"] == 1300
            assert data["path"] == r"C:\repo"
            assert "py" in data["extension_breakdown"]
            assert "(no extension)" in data["extension_breakdown"]
            assert "directories excluded" in data["breakdown_note"]
            # path must be forwarded to every backend call
            assert all(p == r"C:\repo" for p in fb.seen_paths)
        finally:
            server._backend = old_backend
            server._config = old_config

    @pytest.mark.asyncio
    async def test_count_stats_sample_sort(self):
        """sample_sort controls the order used when sampling the breakdown."""
        from everything_mcp import server
        from everything_mcp.config import EverythingConfig
        from everything_mcp.server import CountStatsInput, everything_count_stats

        seen_sorts: list[str] = []

        class FakeBackend:
            async def count(self, query: str, path_filter: str = "") -> int:
                return 1

            async def get_total_size(self, query: str, path_filter: str = "") -> int:
                return 100

            async def search(
                self, query: str, max_results: int, sort: str, path_filter: str = ""
            ):
                seen_sorts.append(sort)
                return [SearchResult(path=r"C:\repo\a.py", name="a.py", size=100)]

        old_backend = server._backend
        old_config = server._config
        try:
            server._backend = FakeBackend()
            server._config = EverythingConfig(es_path=r"C:\Program Files\Everything\es.exe")

            # Default should be date-modified-desc (unbiased sampling).
            await everything_count_stats(
                CountStatsInput(query="*.py", breakdown_by_extension=True)
            )
            assert seen_sorts == ["date-modified-desc"]

            # Custom sort is honoured.
            seen_sorts.clear()
            await everything_count_stats(
                CountStatsInput(query="*.py", breakdown_by_extension=True, sample_sort="size")
            )
            assert seen_sorts == ["size"]

            # Invalid sort is rejected by the validator.
            with pytest.raises(ValueError, match="Invalid sample_sort"):
                CountStatsInput(query="*.py", sample_sort="bogus")
        finally:
            server._backend = old_backend
            server._config = old_config

    @pytest.mark.asyncio
    async def test_search_include_total(self):
        """include_total appends the overall match count to the result text."""
        from everything_mcp import server
        from everything_mcp.config import EverythingConfig
        from everything_mcp.server import SearchInput, everything_search

        class FakeBackend:
            async def search(self, **kwargs):
                return [SearchResult(path=r"C:\repo\a.py", name="a.py", size=1200)]

            async def count(self, query: str, path_filter: str = "") -> int:
                return 42

        old_backend = server._backend
        old_config = server._config
        try:
            server._backend = FakeBackend()
            server._config = EverythingConfig(es_path=r"C:\Program Files\Everything\es.exe")

            result = await everything_search(
                SearchInput(query="*.py", include_total=True)
            )
            assert "Total matches: 42" in result

            # Without include_total there is no count line.
            result2 = await everything_search(SearchInput(query="*.py"))
            assert "Total matches" not in result2
        finally:
            server._backend = old_backend
            server._config = old_config

    @pytest.mark.asyncio
    async def test_find_recent_auto_expand(self):
        """Few results in the period trigger an all-time retry (default on)."""
        from everything_mcp import server
        from everything_mcp.config import EverythingConfig
        from everything_mcp.server import FindRecentInput, everything_find_recent

        queries_seen: list[str] = []

        class FakeBackend:
            async def search(self, query, max_results, sort, path_filter=""):
                queries_seen.append(query)
                # first (period) call returns 2, second (all-time) returns 3
                if "dm:last1day" in query:
                    return [
                        SearchResult(path=r"C:\a.py", name="a.py"),
                        SearchResult(path=r"C:\b.py", name="b.py"),
                    ]
                return [
                    SearchResult(path=r"C:\a.py", name="a.py"),
                    SearchResult(path=r"C:\b.py", name="b.py"),
                    SearchResult(path=r"C:\c.py", name="c.py"),
                ]

        old_backend = server._backend
        old_config = server._config
        try:
            server._backend = FakeBackend()
            server._config = EverythingConfig(es_path=r"C:\Program Files\Everything\es.exe")

            # auto_expand on (default): retries without dm:, label notes it
            result = await everything_find_recent(
                FindRecentInput(period="1day", max_results=5)
            )
            assert "auto-expanded" in result
            assert "3 results" in result
            assert queries_seen == ["dm:last1day", ""] or queries_seen[-1] == ""

            # auto_expand off: no retry, label is plain
            queries_seen.clear()
            result2 = await everything_find_recent(
                FindRecentInput(period="1day", max_results=5, auto_expand=False)
            )
            assert "auto-expanded" not in result2
            assert len(queries_seen) == 1
            assert queries_seen[0] == "dm:last1day"
        finally:
            server._backend = old_backend
            server._config = old_config


class TestToolErrorHandling:
    """Verify that tools return error strings rather than raising."""

    @pytest.mark.asyncio
    async def test_search_returns_error_string(self):
        """When backend is unavailable, search returns an error string."""
        from everything_mcp import server

        # Temporarily set invalid state
        old_backend = server._backend
        old_config = server._config
        try:
            server._backend = None
            server._config = None

            from everything_mcp.server import SearchInput, everything_search

            params = SearchInput(query="*.py")
            result = await everything_search(params)
            assert isinstance(result, str)
            assert "Error" in result
        finally:
            server._backend = old_backend
            server._config = old_config

    @pytest.mark.asyncio
    async def test_count_stats_returns_error_string(self):
        from everything_mcp import server

        old_backend = server._backend
        old_config = server._config
        try:
            server._backend = None
            server._config = None

            from everything_mcp.server import CountStatsInput, everything_count_stats

            params = CountStatsInput(query="*.py")
            result = await everything_count_stats(params)
            assert isinstance(result, str)
            assert "Error" in result
        finally:
            server._backend = old_backend
            server._config = old_config
