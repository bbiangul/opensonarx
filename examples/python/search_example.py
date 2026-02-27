"""
Example: Search the OpenSonarX network using Python bindings.

Requirements:
    pip install opensonarx  # (future PyO3 bindings)

Usage:
    python search_example.py "rust programming"
"""

import sys


def main():
    query = sys.argv[1] if len(sys.argv) > 1 else "decentralized search"

    # Stub: PyO3 bindings not yet available
    print(f"OpenSonarX Python SDK example")
    print(f"Query: {query}")
    print(f"(PyO3 bindings coming soon)")


if __name__ == "__main__":
    main()
