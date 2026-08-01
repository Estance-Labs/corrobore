#!/usr/bin/env python3

"""Load the approved versioned OpenCTI demonstration bundle."""

import json
import logging
import os
from pathlib import Path
import sys


APPROVED_DATASETS = {"corrobore-demo"}
IMPORTER_DIR = Path(
    os.environ.get("OPENCTI_DEMO_IMPORTER_DIR", "/opt/opencti/src/python/testing")
)
DATA_DIR = Path(os.environ.get("OPENCTI_DEMO_DATA_DIR", "/opt/opencti/tests/data"))
API_URL = os.environ.get("OPENCTI_DEMO_API_URL", "http://127.0.0.1:8080")
MAX_IMPORT_PASSES = 3


class ImportErrorCounter(logging.Handler):
    """Track errors swallowed by OpenCTI's object-by-object test importer."""

    def __init__(self) -> None:
        super().__init__(level=logging.ERROR)
        self.error_count = 0

    def emit(self, _record: logging.LogRecord) -> None:
        # Count every importer error so a partial bundle can never look successful.
        self.error_count += 1


def bootstrap_identities(bundle: Path, token: str) -> None:
    """Create identity references before importing objects that depend on them."""
    # Import each STIX identity through the public PyCTI entity helper.
    payload = json.loads(bundle.read_text(encoding="utf-8"))
    identities = [
        item
        for item in payload.get("objects", [])
        if isinstance(item, dict) and item.get("type") == "identity"
    ]
    if not identities:
        return
    from pycti import OpenCTIApiClient

    api_client = OpenCTIApiClient(API_URL, token)
    for identity in identities:
        result = api_client.identity.import_from_stix2(
            stixObject=identity,
            extras={},
            update=True,
        )
        if result is None:
            fail(f"identity bootstrap failed: {identity.get('id', 'unknown')}")


def import_until_clean(importer: object, error_counter: ImportErrorCounter) -> None:
    """Reconcile forward references until one complete import pass is clean."""
    # Reset the pass-local error count, retry idempotently, and fail after the bound.
    for pass_number in range(1, MAX_IMPORT_PASSES + 1):
        error_counter.error_count = 0
        importer.inject()
        if error_counter.error_count == 0:
            return
        if pass_number < MAX_IMPORT_PASSES:
            print(
                f"[OPENCTI] Import pass {pass_number} reported "
                f"{error_counter.error_count} error(s); reconciling",
                file=sys.stderr,
            )
    fail(
        f"OpenCTI import failed after {MAX_IMPORT_PASSES} passes with "
        f"{error_counter.error_count} error(s) in the final pass"
    )


def fail(message: str) -> None:
    raise SystemExit(f"demo data error: {message}")


def selected_datasets() -> list[str]:
    if len(sys.argv) != 2:
        fail("expected one comma-separated dataset selection")
    datasets = sys.argv[1].split(",")
    if not datasets or any(dataset not in APPROVED_DATASETS for dataset in datasets):
        fail(f"unsupported demo dataset selection: {sys.argv[1]}")
    return datasets


def main() -> None:
    token = os.environ["APP__ADMIN__TOKEN"]
    sys.path.insert(0, str(IMPORTER_DIR))
    from local_importer import TestLocalImporter

    datasets = selected_datasets()
    error_counter = ImportErrorCounter()
    root_logger = logging.getLogger()
    root_logger.addHandler(error_counter)
    try:
        for dataset in datasets:
            bundle = DATA_DIR / f"{dataset}.json"
            if not bundle.is_file():
                fail(f"versioned demo bundle is missing: {bundle}")
            bootstrap_identities(bundle, token)
            import_until_clean(TestLocalImporter(API_URL, token, str(bundle)), error_counter)
    finally:
        root_logger.removeHandler(error_counter)
    print(f"[OPENCTI] Dataset insertion succeeded: {','.join(datasets)}")


if __name__ == "__main__":
    main()
