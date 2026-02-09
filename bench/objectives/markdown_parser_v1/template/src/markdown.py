import argparse
import sys
from pathlib import Path


def render_markdown(text: str) -> str:
    """Render a constrained subset of Markdown to HTML.

    The expected behavior is defined in README.md and tests.
    """
    raise NotImplementedError("Implement render_markdown")


def _read_input(path: str | None) -> str:
    if path:
        return Path(path).read_text(encoding="utf-8")
    return sys.stdin.read()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Render markdown to HTML")
    parser.add_argument("path", nargs="?", help="Optional path to markdown input")
    args = parser.parse_args(argv)

    output = render_markdown(_read_input(args.path))
    sys.stdout.write(output)
    if not output.endswith("\n"):
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
