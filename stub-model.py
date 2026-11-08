# A stand-in OpenAI-compatible model for dry runs: answers in the agent's role, no API key needed.
import json, time, random
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def do_POST(self):
        body=json.loads(self.rfile.read(int(self.headers.get('Content-Length',0))) or b'{}')
        msgs=body.get('messages',[]); sys_=next((m['content'] for m in msgs if m['role']=='system'),''); user=msgs[-1]['content'] if msgs else ''
        role=sys_.split('.')[0].replace('You are ','') or 'agent'
        rnd='ROUND 2' if user.startswith('ROUND 2') else 'ROUND 3' if user.startswith('ROUND 3') else 'ROUND 1'
        time.sleep(random.uniform(1.5,4))
        text=f"[{rnd} stand-in reply from the {role}] " + {
          'ROUND 1':"The deck should ask one question per slide and convert every unit to dollars and days. Slide 5 carries two ideas; split it.",
          'ROUND 2':"The other two briefs are right about the close and wrong about slide 5. I would fight for one change: a dated figure on slide 3.",
          'ROUND 3':"Ruling. The policy analyst's theory governs. Changes that survive: slide 3 dated figure; slide 5 split. Rejected: dropping the calculator. Remaining risk: the farm metaphor wears thin by slide 9."}[rnd]
        out={"id":"stub","object":"chat.completion","created":int(time.time()),"model":body.get('model','stub'),
             "choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}],
             "usage":{"prompt_tokens":len(user)//4,"completion_tokens":len(text)//4,"total_tokens":(len(user)+len(text))//4}}
        b=json.dumps(out).encode(); self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(b))); self.end_headers(); self.wfile.write(b)
HTTPServer(('127.0.0.1',8790),H).serve_forever()
