#!/usr/bin/env python3
"""One offline model demonstration on a wholly fictional workload."""
import hashlib
import json
import os
from pathlib import Path
import resource
import socket
import time

BASE = Path(__file__).resolve().parent
MODEL = BASE / 'models' / 'laya-multilingual-mlx'
os.environ['HF_HUB_OFFLINE'] = '1'
os.environ['HF_HUB_DISABLE_TELEMETRY'] = '1'
os.environ['HF_HUB_DISABLE_IMPLICIT_TOKEN'] = '1'
os.environ['HF_HOME'] = str(BASE / 'cache' / 'huggingface')
os.environ['TOKENIZERS_PARALLELISM'] = 'false'
os.environ['TMPDIR'] = str(BASE / 'tmp')

def no_network(*args, **kwargs):
    raise RuntimeError('Network connections disabled in offline demonstration')
socket.socket.connect = no_network
socket.socket.connect_ex = no_network
socket.create_connection = no_network

manifest = json.loads((MODEL / 'manifest.json').read_text())
for name, expected in manifest['files'].items():
    path = MODEL / name
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(4 * 1024 * 1024), b''):
            digest.update(block)
    if path.stat().st_size != expected['bytes'] or digest.hexdigest() != expected['sha256']:
        raise RuntimeError('Checkpoint manifest verification failed: ' + name)

import laya_mlx
import mlx.core as mx

start = time.perf_counter()
model = laya_mlx.load(str(MODEL))
loaded = time.perf_counter()
state = ('Scénario entièrement fictif : une application au premier plan est lente. '
         'Un calcul de fond utilise fortement le processeur. '
         'La mémoire disponible est suffisante et il n’y a pas de pression mémoire.')
questions = {'action': {'type': 'choice',
    'instructions': 'Quelle action réversible favorise la réactivité sans traiter une pression mémoire inexistante ?',
    'criteria': {
        'foreground': 'Donner une préférence temporaire à la tâche interactive et garder du temps au travail de fond.',
        'compress': 'Augmenter la compression mémoire alors que la mémoire disponible est suffisante.',
        'wait': 'Ne faire aucun ajustement et continuer à partager le processeur comme avant.'
    }}}
result = model.predict(state, questions)
end = time.perf_counter()
output = {
    'model': 'aac6fef/laya-multilingual-mlx',
    'model_revision': 'f2b4faf51023039425946074e2cf1361d2db11d5',
    'model_sha256': manifest['files']['model.safetensors']['sha256'],
    'software': {'laya_mlx': '0.6.0', 'mlx': '0.32.4'},
    'environment': {'host': 'Apple M4 Pro, macOS 15.1.1', 'offline_mode': True,
                    'network_guard': 'macOS sandbox denies network; Python socket connect disabled' if os.getenv('LOCAL_DECISION_OS_SANDBOX') == '1' else 'Python socket connect disabled', 'source_state': 'fictional'},
    'manifest_integrity_ok': True,
    'load_seconds': loaded - start,
    'first_inference_seconds': end - loaded,
    'max_rss_bytes': resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
    'mlx_peak_bytes': mx.get_peak_memory(),
    'state': state,
    'questions': questions,
    'result': result,
    'notes': ['One example does not establish task accuracy or calibration.',
              'The score is the model output, not an independently established probability of correctness.',
              'This is Laya, a separate model, not TypeSafe Jev or a proven equivalent.',
              'No system policy was changed.']
}
out_dir = BASE / 'run-results'
out_dir.mkdir(exist_ok=True)
(out_dir / 'demo-result.json').write_text(json.dumps(output, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(output, ensure_ascii=False, indent=2))
