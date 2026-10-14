"""Concatenate src/*.txt into dist/out.txt."""
from pathlib import Path

out = Path("dist")
out.mkdir(exist_ok=True)
(out / "out.txt").write_text("".join(p.read_text() for p in sorted(Path("src").glob("*.txt"))))
print("built dist/out.txt")
