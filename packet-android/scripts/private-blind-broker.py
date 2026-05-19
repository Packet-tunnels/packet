#!/usr/bin/env python3
"""
Private blind Packet broker for an Iran VPS.

This is intentionally Python-only and dependency-free so the VPS does not need
Rust, git, or the Packet source tree. It does NOT know the Packet private key.
It only pairs one phone WebSocket with one Starlink relay WebSocket and forwards
opaque encrypted binary frames between them.

Security model:
  - VPS provider can see client IPs, relay IP, timing, and byte counts.
  - VPS provider cannot read destinations or payloads unless they also get the
    private key from the phone/Starlink laptop.
  - One active client is allowed per broker to avoid blind routing ambiguity.

Run on Iran VPS:
  python3 private-blind-broker.py --port 80

Then use the printed Private Key on both:
  - Android app: Stack = Private Relay, Server URL = http://IRAN_VPS_IP:80
  - Starlink laptop: phantom-relay --server http://IRAN_VPS_IP:80 --secret KEY
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import hashlib
import json
import os
import secrets
import signal
import struct
import sys
import time
from dataclasses import dataclass
from typing import Optional

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
MAX_FRAME = 8 * 1024 * 1024


@dataclass
class Peer:
    role: str
    reader: asyncio.StreamReader
    writer: asyncio.StreamWriter
    label: str
    connected_at: float
    bytes_in: int = 0
    bytes_out: int = 0

    def closed(self) -> bool:
        return self.writer.is_closing()


class BlindBroker:
    def __init__(self, private_key: str):
        self.private_key = private_key
        # Public room id is not a secret. It is printed only for operator sanity.
        self.room_id = hashlib.sha256(private_key.encode()).hexdigest()[:16]
        self.relay: Optional[Peer] = None
        self.client: Optional[Peer] = None
        self.lock = asyncio.Lock()

    async def handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        peername = writer.get_extra_info("peername")
        try:
            headers = await self.read_http_headers(reader)
            if not headers:
                return
            method, path, header_map = headers

            if method == "GET" and path.startswith("/api/v1/health"):
                await self.write_health(writer)
                return

            if header_map.get("upgrade", "").lower() != "websocket":
                await self.write_http(writer, 404, b"not found\n", "text/plain")
                return

            ws_key = header_map.get("sec-websocket-key")
            if not ws_key:
                await self.write_http(writer, 400, b"missing websocket key\n", "text/plain")
                return

            await self.accept_websocket(writer, ws_key)
            auth_opcode, auth_payload = await asyncio.wait_for(read_ws_frame(reader), timeout=10)
            if auth_opcode != 0x1:
                await send_ws_text(writer, json.dumps({"error": "expected auth text"}))
                return

            role, label = self.classify_auth(auth_payload)
            peer = Peer(role=role, reader=reader, writer=writer, label=label, connected_at=time.time())

            if role == "relay":
                await self.register_relay(peer)
                await send_ws_text(writer, json.dumps({"relay_id": f"blind-{self.room_id}", "status": "accepted"}))
                print(f"[broker] relay accepted label={label} peer={peername}", flush=True)
                await self.pipe_from_relay(peer)
            else:
                accepted = await self.register_client(peer)
                if not accepted:
                    return
                await send_ws_text(writer, json.dumps({"token": f"blind-{self.room_id}"}))
                print(f"[broker] client accepted label={label} peer={peername}", flush=True)
                await self.pipe_from_client(peer)
        except asyncio.IncompleteReadError:
            pass
        except Exception as exc:
            print(f"[broker] connection error peer={peername}: {exc}", file=sys.stderr, flush=True)
        finally:
            await self.unregister(writer)
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass

    @staticmethod
    async def read_http_headers(reader: asyncio.StreamReader):
        raw = await reader.readuntil(b"\r\n\r\n")
        text = raw.decode("iso-8859-1", errors="replace")
        lines = text.split("\r\n")
        if not lines or " " not in lines[0]:
            return None
        parts = lines[0].split()
        method = parts[0].upper()
        path = parts[1] if len(parts) > 1 else "/"
        headers = {}
        for line in lines[1:]:
            if not line or ":" not in line:
                continue
            name, value = line.split(":", 1)
            headers[name.strip().lower()] = value.strip()
        return method, path, headers

    async def write_health(self, writer: asyncio.StreamWriter) -> None:
        body = json.dumps(
            {
                "status": "ok",
                "mode": "private-blind-broker",
                "room": self.room_id,
                "relay": bool(self.relay and not self.relay.closed()),
                "client": bool(self.client and not self.client.closed()),
                "service": "packet-private-blind",
            }
        ).encode()
        await self.write_http(writer, 200, body, "application/json")

    @staticmethod
    async def write_http(writer: asyncio.StreamWriter, status: int, body: bytes, content_type: str) -> None:
        reason = {200: "OK", 400: "Bad Request", 404: "Not Found", 503: "Unavailable"}.get(status, "OK")
        writer.write(
            f"HTTP/1.1 {status} {reason}\r\n"
            f"Content-Type: {content_type}\r\n"
            f"Content-Length: {len(body)}\r\n"
            "Connection: close\r\n"
            "\r\n".encode()
            + body
        )
        await writer.drain()

    @staticmethod
    async def accept_websocket(writer: asyncio.StreamWriter, ws_key: str) -> None:
        accept = base64.b64encode(hashlib.sha1((ws_key + GUID).encode()).digest()).decode()
        writer.write(
            "HTTP/1.1 101 Switching Protocols\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Accept: {accept}\r\n"
            "\r\n".encode()
        )
        await writer.drain()

    @staticmethod
    def classify_auth(payload: bytes) -> tuple[str, str]:
        try:
            data = json.loads(payload.decode())
        except Exception:
            return "client", "unknown"
        role = "relay" if data.get("mode") == "relay" else "client"
        label = str(data.get("label") or data.get("mode") or role)
        return role, label[:80]

    async def register_relay(self, peer: Peer) -> None:
        async with self.lock:
            old = self.relay
            self.relay = peer
            if old and not old.closed():
                old.writer.close()

    async def register_client(self, peer: Peer) -> bool:
        async with self.lock:
            if not self.relay or self.relay.closed():
                await send_ws_text(peer.writer, json.dumps({"error": "no Starlink relay connected"}))
                return False
            if self.client and not self.client.closed():
                await send_ws_text(peer.writer, json.dumps({"error": "broker busy: one active client allowed"}))
                return False
            self.client = peer
            return True

    async def unregister(self, writer: asyncio.StreamWriter) -> None:
        async with self.lock:
            if self.client and self.client.writer is writer:
                print("[broker] client disconnected", flush=True)
                self.client = None
            if self.relay and self.relay.writer is writer:
                print("[broker] relay disconnected", flush=True)
                self.relay = None
                if self.client and not self.client.closed():
                    await send_ws_close(self.client.writer)
                    self.client.writer.close()
                    self.client = None

    async def pipe_from_client(self, peer: Peer) -> None:
        while True:
            opcode, payload = await read_ws_frame(peer.reader)
            if opcode == 0x8:
                break
            if opcode == 0x9:
                await send_ws_frame(peer.writer, 0xA, payload)
                continue
            if opcode != 0x2:
                continue
            async with self.lock:
                relay = self.relay
            if not relay or relay.closed():
                await send_ws_close(peer.writer)
                break
            peer.bytes_in += len(payload)
            relay.bytes_out += len(payload)
            await send_ws_frame(relay.writer, 0x2, payload)

    async def pipe_from_relay(self, peer: Peer) -> None:
        while True:
            opcode, payload = await read_ws_frame(peer.reader)
            if opcode == 0x8:
                break
            if opcode == 0x9:
                await send_ws_frame(peer.writer, 0xA, payload)
                continue
            if opcode != 0x2:
                continue
            async with self.lock:
                client = self.client
            if not client or client.closed():
                continue
            peer.bytes_in += len(payload)
            client.bytes_out += len(payload)
            await send_ws_frame(client.writer, 0x2, payload)


async def read_ws_frame(reader: asyncio.StreamReader) -> tuple[int, bytes]:
    head = await reader.readexactly(2)
    opcode = head[0] & 0x0F
    masked = bool(head[1] & 0x80)
    length = head[1] & 0x7F
    if length == 126:
        length = struct.unpack("!H", await reader.readexactly(2))[0]
    elif length == 127:
        length = struct.unpack("!Q", await reader.readexactly(8))[0]
    if length > MAX_FRAME:
        raise ValueError(f"websocket frame too large: {length}")
    mask = await reader.readexactly(4) if masked else b""
    payload = await reader.readexactly(length) if length else b""
    if masked:
        payload = bytes(byte ^ mask[i % 4] for i, byte in enumerate(payload))
    return opcode, payload


async def send_ws_text(writer: asyncio.StreamWriter, text: str) -> None:
    await send_ws_frame(writer, 0x1, text.encode())


async def send_ws_close(writer: asyncio.StreamWriter) -> None:
    await send_ws_frame(writer, 0x8, b"")


async def send_ws_frame(writer: asyncio.StreamWriter, opcode: int, payload: bytes) -> None:
    length = len(payload)
    header = bytearray([0x80 | opcode])
    if length < 126:
        header.append(length)
    elif length <= 0xFFFF:
        header.extend([126])
        header.extend(struct.pack("!H", length))
    else:
        header.extend([127])
        header.extend(struct.pack("!Q", length))
    writer.write(bytes(header) + payload)
    await writer.drain()


async def main() -> None:
    parser = argparse.ArgumentParser(description="Packet private blind broker")
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=80)
    parser.add_argument("--key", default=os.environ.get("PACKET_PRIVATE_KEY"))
    parser.add_argument("--self-delete", action="store_true", help="unlink this script file after startup on Linux")
    args = parser.parse_args()

    private_key = args.key or secrets.token_urlsafe(32)
    broker = BlindBroker(private_key)

    if args.self_delete:
        try:
            os.unlink(__file__)
            print("[broker] script file unlinked; process keeps running from memory", flush=True)
        except Exception as exc:
            print(f"[broker] self-delete failed: {exc}", file=sys.stderr, flush=True)

    server = await asyncio.start_server(broker.handle, args.host, args.port)
    sockets = ", ".join(str(sock.getsockname()) for sock in server.sockets or [])

    print()
    print("Packet Private Blind Broker is running")
    print(f"Listen      : {sockets}")
    print(f"Room ID     : {broker.room_id} (not secret)")
    print(f"Private Key : {private_key}")
    print()
    print("Android app:")
    print(f"  Stack      : Private Relay")
    print(f"  Server URL : http://YOUR_IRAN_VPS_IP:{args.port}")
    print(f"  Secret     : {private_key}")
    print()
    print("Starlink laptop:")
    print(f"  PHANTOM_SERVER=http://YOUR_IRAN_VPS_IP:{args.port} PHANTOM_SECRET='{private_key}' \\")
    print("    target/release/phantom-relay --label starlink-laptop")
    print()
    print("Keep this terminal open. Ctrl+C stops the broker.")
    print()
    sys.stdout.flush()

    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, stop.set)
        except NotImplementedError:
            pass

    async with server:
        await stop.wait()


if __name__ == "__main__":
    asyncio.run(main())
