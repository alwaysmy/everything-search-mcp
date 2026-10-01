#!/usr/bin/env python3
"""Exercise the compiled CLI and MCP binary against an isolated HTTP fixture.

Usage: python3 TEST_SCRIPTS/mock-http-smoke.py --binary target/release/everything-search-mcp
Optional: --validate-schema additionally requires the Python jsonschema package.
No real Everything index, registry or user files are read or modified.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import urllib.parse


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--binary', required=True)
    ap.add_argument('--validate-schema', action='store_true')
    args = ap.parse_args()
    binary = str(Path(args.binary).resolve())
    rows = [dict(name='a.txt', path='C:\\A', type='file', size='1'),
            dict(name='dir', path='C:\\A', type='folder'),
            dict(name='c.txt', path='C:\\B', type='file', size='3'),
            dict(name='d.txt', path='C:\\C', type='file', size='4')]

    class Fixture(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
            offset, count = int(query['offset'][0]), int(query['count'][0])
            body = json.dumps(dict(totalResults=len(rows), results=rows[offset:offset+count])).encode()
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        url = f'http://127.0.0.1:{server.server_port}'
        with socket.socket() as reserved:
            reserved.bind(('127.0.0.1', 0))
            dead = f'http://127.0.0.1:{reserved.getsockname()[1]}'
        with tempfile.TemporaryDirectory(prefix='everything-mock-') as temp:
            registry = Path(temp) / 'servers.json'
            registry.write_text('{}')
            local_file = Path(temp) / 'sample.txt'
            local_file.write_text('fixture only\n')
            env = dict(os.environ, EVERYTHING_SERVERS_FILE=str(registry), EVERYTHING_TIMEOUT='1')
            search_args = dict(query='*', url=url, max_results=2, max_per_parent=1)
            calls = [
                ('everything_search', search_args),
                ('everything_find_recent', dict(url=url, auto_expand=False)),
                ('everything_count_stats', dict(query='*', url=url, exact_size=True, breakdown_by_extension=True)),
                ('everything_search_batch', dict(max_total_results=1, queries=[dict(query='*', url=[url, dead]), search_args])),
                ('everything_file_details', dict(paths=[str(local_file)])),
                ('everything_search', dict(query='*', url=[url, dead])),
            ]
            requests = [dict(jsonrpc='2.0', id=1, method='initialize', params={}),
                        dict(jsonrpc='2.0', id=2, method='tools/list', params={})]
            requests += [dict(jsonrpc='2.0', id=i+3, method='tools/call',
                              params=dict(name=name, arguments=arguments)) for i, (name, arguments) in enumerate(calls)]
            run = subprocess.run([binary], input=''.join(json.dumps(r)+'\n' for r in requests),
                                 env=env, text=True, capture_output=True, timeout=15, check=True)
            replies = {r['id']: r for r in map(json.loads, run.stdout.splitlines())}
            assert len(replies) == len(requests), run.stdout
            schemas = {t['name']: t['outputSchema'] for t in replies[2]['result']['tools']}
            assert len(schemas) == 5
            outputs = []
            for i, (name, _) in enumerate(calls):
                result = replies[i+3]['result']
                assert not result['isError'], result
                output = result['structuredContent']
                if args.validate_schema:
                    import jsonschema
                    jsonschema.validate(output, schemas[name])
                outputs.append(output)
            assert outputs[0]['next_offset'] == 3
            assert outputs[2]['total_size'] == 8
            assert outputs[3]['total_returned'] == 1
            assert outputs[3]['queries_skipped'] == 1
            assert outputs[3]['results'][0]['total_accuracy'] == 'partial'
            assert 'error' in outputs[3]['results'][0]['backends'][1]
            assert outputs[5]['total_scope'] == 'reachable indexes'
            cli = subprocess.run([binary, 'search', '*', '--url', url, '--max', '2', '--per-parent', '1', '--json'],
                                 env=env, text=True, capture_output=True, timeout=5, check=True)
            cli_result = json.loads(cli.stdout)
            mcp_result = dict(outputs[0])
            cli_result.pop('elapsed_ms'); mcp_result.pop('elapsed_ms')
            assert cli_result == mcp_result, (cli_result, mcp_result)
            print('PASS: CLI/MCP parity, five tool schemas, pagination, size sum, partial backend status and hard batch budget')
            if args.validate_schema:
                print('PASS: six structured responses validate against their published output schemas')
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
