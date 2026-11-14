"""Local OpenAI-compatible SSE mock used for end-to-end testing.

Run: python tools/mock_sse.py
Listens on http://127.0.0.1:8765/chat/completions
"""

import http.server
import json
import time


CHUNKS = [
    "你好！",
    "这是一条",
    "来自本地 mock 服务的",
    "流式翻译结果。",
    "\n\n**完成**",
]


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(length)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        for chunk in CHUNKS:
            payload = json.dumps(
                {"choices": [{"delta": {"content": chunk}}]}, ensure_ascii=False
            )
            self.wfile.write(f"data: {payload}\n\n".encode("utf-8"))
            self.wfile.flush()
            time.sleep(0.12)
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    http.server.HTTPServer(("127.0.0.1", 8765), Handler).serve_forever()
