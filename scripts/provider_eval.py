#!/usr/bin/env python3
"""Measure the provider routing table from data instead of guessing.

Thin entry point; the code lives in scripts/provider_eval/ (see its cli module for the
subcommands). Run with `python3 -I scripts/provider_eval.py ...`.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from provider_eval.cli import main  # noqa: E402

if __name__ == "__main__":
    main()
