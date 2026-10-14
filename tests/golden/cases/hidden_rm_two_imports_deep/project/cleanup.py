"""Remove build artifacts."""
from tools import fs

if __name__ == "__main__":
    fs.tidy("build")
    print("done")
