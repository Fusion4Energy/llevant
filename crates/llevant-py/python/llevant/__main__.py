import sys

from ._llevant import main

if __name__ == "__main__":
    sys.exit(main(["llevant", *sys.argv[1:]]))
