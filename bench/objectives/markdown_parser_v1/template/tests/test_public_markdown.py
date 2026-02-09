import subprocess
import sys

from src.markdown import render_markdown


def test_heading_paragraph_and_escaping() -> None:
    text = "# Title\n\nHello <world> & all."
    expected = "<h1>Title</h1>\n<p>Hello &lt;world&gt; &amp; all.</p>"
    assert render_markdown(text) == expected


def test_unordered_list_block() -> None:
    text = "- one\n- two\n\nafter"
    expected = "<ul>\n<li>one</li>\n<li>two</li>\n</ul>\n<p>after</p>"
    assert render_markdown(text) == expected


def test_fenced_code_block() -> None:
    text = "```\na < b && c\n```\n"
    expected = "<pre><code>a &lt; b &amp;&amp; c\n</code></pre>"
    assert render_markdown(text) == expected


def test_inline_features() -> None:
    text = "Use **bold**, *italic*, and `code` with [docs](https://example.com)."
    expected = (
        "<p>Use <strong>bold</strong>, <em>italic</em>, and <code>code</code> "
        '<a href="https://example.com">docs</a>.</p>'
    )
    assert render_markdown(text) == expected


def test_cli_reads_stdin_and_prints_html() -> None:
    proc = subprocess.run(
        [sys.executable, "-m", "src.markdown"],
        input="# H\n",
        text=True,
        capture_output=True,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.strip() == "<h1>H</h1>"
