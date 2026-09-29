#!/usr/bin/env python3
"""Packaged plugin wire smoke; Caffeinate starts and stops a short local session."""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import queue
import subprocess
import threading
import time
from pathlib import Path

FEATURES = ['action.arguments', 'configuration.values', 'host_action.object', 'search.cancel', 'status.snapshots']
SERVICES = {'system.media': 1, 'text.edit': 1, 'notifications.authorize': 1, 'menu.press': 1}

class Client:
    def __init__(self, executable):
        self.process = subprocess.Popen([str(executable)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.events = queue.Queue()
        def read():
            for line in self.process.stdout:
                try:
                    self.events.put(json.loads(line))
                except ValueError as error:
                    self.events.put(error)
        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()

    def send(self, **request):
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()

    def receive(self, kind, timeout=5):
        deadline = time.monotonic() + timeout
        while True:
            event = self.events.get(timeout=max(0.001, deadline - time.monotonic()))
            if isinstance(event, Exception):
                raise event
            if event['type'] == kind:
                return event
            if event['type'] == 'error':
                raise AssertionError(event)

    def close(self):
        try:
            self.send(type='shutdown')
            self.process.wait(timeout=2)
        except (BrokenPipeError, subprocess.TimeoutExpired):
            self.process.kill()
            self.process.wait()
        finally:
            self.process.stdin.close()
            self.reader.join(timeout=2)
            self.process.stdout.close()


def binary(bundle):
    candidates = list(bundle.glob('Contents/MacOS/*'))
    assert len(candidates) == 1, bundle
    return candidates[0]


def cancellation_smoke(app):
    started = threading.Event()
    release = threading.Event()
    class Handler(BaseHTTPRequestHandler):
        def do_HEAD(self):
            started.set()
            release.wait(timeout=3)
            try:
                self.send_response(200)
                self.end_headers()
            except BrokenPipeError:
                pass
        def log_message(self, *_args):
            pass
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    client = Client(binary(app / 'Contents/PlugIns/io.clipkeep.http-inspector.clipkeepplugin'))
    try:
        client.send(type='initialize', protocol_version=1, host_features=FEATURES, host_services=SERVICES)
        assert 'search.cancel' in client.receive('initialized')['features']
        client.send(type='search', query_id=42, query=f'http http://127.0.0.1:{server.server_port}', limit=10, timeout_ms=2000)
        assert started.wait(timeout=2), 'local HTTP probe did not start'
        before = time.monotonic()
        client.send(type='cancel_search', query_id=42)
        assert client.receive('error', timeout=1)['query_id'] == 42
        assert time.monotonic() - before < 1
        client.send(type='search', query_id=43, query='ordinary search', limit=10, timeout_ms=500)
        assert client.receive('search_results', timeout=1)['query_id'] == 43
        print('PASS live HTTP cancellation and following query (loopback only)')
    finally:
        release.set()
        client.close()
        server.shutdown()
        server.server_close()
        worker.join(timeout=2)


def smoke(app, hello):
    for modern in (False, True):
        plugin = app / 'Contents/PlugIns/io.clipkeep.calculator.clipkeepplugin'
        client = Client(binary(plugin))
        try:
            request = dict(type='initialize', protocol_version=1)
            if modern:
                request.update(host_features=FEATURES, host_services=SERVICES)
            client.send(**request)
            ready = client.receive('initialized')
            assert ready['plugin_id'] == 'io.clipkeep.calculator'
            client.send(type='configure', settings={})
            client.send(type='search', query_id=1, query='2+3', limit=1, timeout_ms=1000)
            event = client.receive('search_results')
            assert event['query_id'] == 1 and len(event['items']) == 1
            assert event['items'][0]['title'] == '5'
            print(f"PASS calculator {'modern' if modern else 'legacy'} handshake/search")
        finally:
            client.close()
    if hello:
        client = Client(binary(hello))
        try:
            client.send(type='initialize', protocol_version=1, host_features=FEATURES, host_services=SERVICES)
            ready = client.receive('initialized')
            assert ready['configuration_schema'][0]['id'] == 'greeting'
            client.send(type='configure', settings={}, values={'greeting': 'Protocol smoke'})
            client.send(type='search', query_id=2, query='hello', limit=10, timeout_ms=1000)
            event = client.receive('search_results')
            assert event['items'][0]['title'] == 'Protocol smoke from a plugin'
            client.send(type='execute', request_id=3, result_id='hello', action_id='hello', arguments={})
            assert client.receive('action_finished')['success']
            print('PASS independently packaged Hello schema/configure/arguments/action')
        finally:
            client.close()
    client = Client(binary(app / 'Contents/PlugIns/io.clipkeep.ocr.clipkeepplugin'))
    try:
        client.send(type='initialize', protocol_version=1, host_features=FEATURES, host_services=SERVICES)
        assert client.receive('initialized')['plugin_id'] == 'io.clipkeep.ocr'
        client.send(type='search', query_id=4, query='ocr', limit=10)
        assert client.receive('search_results')['query_id'] == 4
        print('PASS native Swift OCR plugin handshake/search (no clipboard access action)')
    finally:
        client.close()


def status_smoke(app):
    client = Client(binary(app / 'Contents/PlugIns/io.clipkeep.caffeinate.clipkeepplugin'))
    try:
        client.send(type='initialize', protocol_version=1, host_features=FEATURES, host_services=SERVICES)
        assert 'status.snapshots' in client.receive('initialized')['features']
        assert client.receive('status_changed')['presentation_items'] == []
        client.send(type='execute', request_id=10, result_id='caffeinate:start', action_id='start',
                    arguments={'seconds': 15})
        assert client.receive('action_finished')['success']
        active = client.receive('status_changed')['presentation_items']
        assert len(active) == 1 and int(active[0]['metadata']['countdown_until']) > time.time()
        assert active[0]['compact_value'] == 'Time left'
        client.send(type='execute', request_id=11, result_id='caffeinate:stop', action_id='stop')
        assert client.receive('action_finished')['success']
        assert client.receive('status_changed')['presentation_items'] == []
        print('PASS packaged Caffeinate status start/clear without search')
    finally:
        client.close()

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('app', type=Path)
    parser.add_argument('--hello', type=Path)
    parser.add_argument('--cancellation', action='store_true', help='test cancellation using a local HTTP fixture')
    args = parser.parse_args()
    smoke(args.app.resolve(), args.hello.resolve() if args.hello else None)
    status_smoke(args.app.resolve())
    if args.cancellation:
        cancellation_smoke(args.app.resolve())
