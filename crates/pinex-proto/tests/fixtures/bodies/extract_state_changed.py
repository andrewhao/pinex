import re, sys

src = open('/private/tmp/claude-501/-Users-andrewhao-workspace-pinex/e97abda2-3f7c-421f-8ce5-3aac3345a87c/scratchpad/protocol.md').read()
lines = src.splitlines()

start = next(i for i,l in enumerate(lines) if l.startswith('### State Changed'))
fence = [i for i,l in enumerate(lines[start:start+20], start) if l.strip()=='```'][0]
end = next(i for i,l in enumerate(lines[fence+1:], fence+1) if l.strip()=='```')

body = []
for l in lines[fence+1:end]:
    l = re.sub(r'\[[^\]]*\]', ' ', l)          # drop prose annotations
    for tok in l.split():
        if re.fullmatch(r'[0-9a-fA-F]{2}', tok):
            body.append(int(tok, 16))

data = bytes(body)
print("total bytes:", len(data))
print("first 16:", data[:16].hex(' '))
print("last 16:", data[-16:].hex(' '))
open('/private/tmp/claude-501/-Users-andrewhao-workspace-pinex/e97abda2-3f7c-421f-8ce5-3aac3345a87c/scratchpad/state_changed.body.bin','wb').write(data)

# Verify: header, then the first value must be the annotated inputTrim 15.0
idx = data.find(bytes.fromhex('88 00 00 70 41'))
print("inputTrim 88 00 00 70 41 at index:", idx)
