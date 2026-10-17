#!/usr/bin/env python3
"""Opt-in native GUI + stdio MCP smoke test; uses a temporary SQLite workspace.

Run after cargo build -p glint-api. GLINT_API_BINARY overrides the debug binary.
Requires a desktop session; never connects to the user's existing workspace.
"""
import os, json, time, tempfile, subprocess, threading, hashlib, base64, struct, selectors, sqlite3
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from pathlib import Path

class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *args): pass
    def do_GET(self):
        if self.path == '/ws':
            accept = base64.b64encode(hashlib.sha1((self.headers['Sec-WebSocket-Key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
            self.send_response(101); self.send_header('Upgrade','websocket'); self.send_header('Connection','Upgrade'); self.send_header('Sec-WebSocket-Accept',accept); self.end_headers()
            self.connection.settimeout(10)
            try:
                while True:
                    header=self.rfile.read(2)
                    if len(header)!=2:break
                    op=header[0]&15; length=header[1]&127
                    if length==126:length=struct.unpack('!H', self.rfile.read(2))[0]
                    if length==127:length=struct.unpack('!Q', self.rfile.read(8))[0]
                    mask=self.rfile.read(4) if header[1]&128 else b''
                    payload=self.rfile.read(length)
                    if mask:payload=bytes(v^mask[i%4] for i,v in enumerate(payload))
                    code=8 if op==8 else 10 if op==9 else 1
                    self.wfile.write(bytes([0x80|code,len(payload)])+payload);self.wfile.flush()
                    if op==8:break
            except (OSError, ValueError):pass
            self.close_connection=True
        elif self.path == '/sse':
            self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Connection','close');self.end_headers()
            self.wfile.write('id: 1\nevent: message\ndata: 你好 MCP\n\n'.encode());self.wfile.flush()
            stopped.wait(10);self.close_connection=True
        else:
            payload=json.dumps({'ok':True, 'path': self.path}).encode()
            self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
    def do_POST(self):
        payload=self.rfile.read(int(self.headers.get('Content-Length',0)))
        self.send_response(201);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)

stopped=threading.Event(); server=ThreadingHTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
base=f'http://127.0.0.1:{server.server_port}'; binary=os.environ.get('GLINT_API_BINARY', str(Path(__file__).resolve().parents[3] / 'target/debug/glint-api'))
with tempfile.TemporaryDirectory(prefix='glint-mcp-smoke-') as temp:
    env=os.environ.copy();env['GLINT_API_DB_PATH']=temp+'/workspace.sqlite3'
    host_log=open(temp+'/host.log','w+')
    host=subprocess.Popen([binary],env=env,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=host_log)
    client=None
    try:
        until=time.monotonic()+20
        while not Path(temp+'/workspace.mcp.json').exists():
            if host.poll() is not None:
                host_log.seek(0);raise RuntimeError('GUI exited: '+host_log.read()[-2000:])
            if time.monotonic()>until:raise TimeoutError('GUI discovery not created')
            time.sleep(.1)
        env['GLINT_API_MCP_NO_AUTOSTART']='1'
        client=subprocess.Popen([binary,'--mcp'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,bufsize=1)
        seq=0
        def rpc(method,params=None,notify=False):
            global seq
            seq+=1;msg={'jsonrpc':'2.0','method':method}
            if not notify:msg['id']=seq
            if params is not None:msg['params']=params
            client.stdin.write(json.dumps(msg,ensure_ascii=False)+'\n');client.stdin.flush()
            if notify:return
            selector=selectors.DefaultSelector();selector.register(client.stdout,selectors.EVENT_READ)
            if not selector.select(15):raise TimeoutError('MCP did not reply to '+method)
            selector.close();line=client.stdout.readline()
            if not line:raise RuntimeError('MCP closed: '+client.stderr.read())
            reply=json.loads(line);assert reply['id']==seq,reply
            if 'error' in reply:raise RuntimeError(reply['error'])
            return reply['result']
        def tool(operation,**args):
            result=rpc('tools/call',{'name':'glint_'+operation,'arguments':args})
            if result.get('isError'):raise RuntimeError(operation+': '+str(result['structuredContent']))
            return result['structuredContent']['result']
        rpc('initialize',{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'native-smoke','version':'1'}})
        rpc('notifications/initialized',notify=True)
        count=len(rpc('tools/list')['tools']);assert count>=50
        tool('collection_create',name='MCP smoke')
        tool('environment_create',name='Local')
        tool('environment_variable_set',environment='Local',name='base_url',value=base)
        request=tool('request_create',collection='MCP smoke',name='HTTP echo',draft={'method':'POST','url':'{{base_url}}/echo','body':'{"hello":"MCP"}'})['request_id']
        tool('request_send',request_id=request,environment='Local')
        until=time.monotonic()+10
        while tool('request_status',request_id=request)['running']:
            assert time.monotonic()<until;time.sleep(.05)
        response=tool('response_read',request_id=request);assert response['status']==201 and 'MCP' in response['text'],response
        record=tool('request_get',request_id=request)['history'][0]['id']
        assert tool('response_read',request_id=request,record_id=record)['status']==201
        ws=tool('request_create',collection='MCP smoke',name='WS echo',draft={'method':'WS','url':base.replace('http://','ws://')+'/ws'})['request_id']
        tool('request_send',request_id=ws)
        until=time.monotonic()+10
        while not tool('request_status',request_id=ws)['websocket_connected']:
            assert time.monotonic()<until;time.sleep(.05)
        tool('websocket_send',request_id=ws,text='你好 agent')
        until=time.monotonic()+10
        while not any('接收' in item['message'] and '你好 agent' in item['message'] for item in tool('websocket_read',request_id=ws)['messages']):
            assert time.monotonic()<until;time.sleep(.05)
        tool('request_cancel',request_id=ws)
        until=time.monotonic()+10
        while tool('request_status',request_id=ws)['running']:
            assert time.monotonic()<until;time.sleep(.05)
        sse=tool('request_create',collection='MCP smoke',name='SSE stream',draft={'url':base+'/sse','method':'GET'})['request_id']
        tool('request_send',request_id=sse)
        until=time.monotonic()+10
        while True:
            status=tool('request_status',request_id=sse)
            if status['sse'] and status['bytes']>0:break
            assert time.monotonic()<until;time.sleep(.05)
        assert status['running'],status
        assert '你好 MCP' in tool('response_read',request_id=sse)['text']
        tool('request_cancel',request_id=sse)
        assert not tool('request_status',request_id=sse)['running']
        assert '你好 MCP' in tool('response_read',request_id=sse)['text']
        tool('version_save',request_id=sse,tag='smoke')
        assert len(tool('workspace_get')['requests'])==3
        # Rejected protocol calls must be audited without running their operations.
        for invalid in [
            {'name':'glint_not_a_tool','arguments':{}},
            {'name':'glint_environment_variable_set','arguments':[]},
        ]:
            try:
                rpc('tools/call',invalid)
                raise AssertionError('invalid tool call unexpectedly succeeded')
            except RuntimeError:
                pass
        calls=tool('mcp_history_list',outcome='error',query='not_a_tool')
        assert calls['total']==1,calls
        detail=tool('mcp_history_get',id=calls['calls'][0]['id'])
        assert not detail['summary']['success']
        assert 'Unknown tool' in detail['result']['error']
        tool('ui_set',section='mcp')

        with sqlite3.connect(env['GLINT_API_DB_PATH']) as database:
            saved=json.loads(database.execute('SELECT data FROM workspace WHERE id=1').fetchone()[0])
        assert {item['id'] for item in saved['requests']} == {request,ws,sse}
        index=next(i for i,item in enumerate(saved['requests']) if item['id']==sse)
        assert '你好 MCP' in saved['responses'][index]['text']
        assert saved['requests'][index]['versions'][0]['tag']=='smoke'
        with sqlite3.connect(env['GLINT_API_DB_PATH']) as database:
            assert database.execute('SELECT COUNT(*) FROM mcp_calls').fetchone()[0]>10
            assert database.execute('SELECT COUNT(*) FROM mcp_calls WHERE success=0').fetchone()[0]>=2
        print(json.dumps({'tools':count,'native_gui':True,'stdio_protocol':True,'http_and_history':True,'websocket_echo':True,'sse_stream_and_cancel':True,'workspace_persistence':True,'mcp_history_and_rejections':True}))
    finally:
        stopped.set()
        if client:
            client.stdin.close()
            try:client.wait(timeout=3)
            except subprocess.TimeoutExpired:client.terminate();client.wait(timeout=3)
        host.terminate()
        try:host.wait(timeout=3)
        except subprocess.TimeoutExpired:host.kill();host.wait()
        host_log.close()
server.shutdown();server.server_close()
