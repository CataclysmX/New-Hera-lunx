#!/usr/bin/env python3
"""Parse YAML files and print their top-level keys (syntax check for workflows).
Usage: python3 -I tools/check_yaml.py file.yml...
"""
import sys

import yaml

for p in sys.argv[1:]:
    doc = yaml.safe_load(open(p))
    print(p, 'OK', sorted(str(k) for k in doc), 'jobs:', list(doc.get('jobs', {})))
