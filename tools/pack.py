"""Crée MyStream.streamDeckPlugin : une archive du dossier du plugin, installée par un double-clic."""
import os
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NAME = 'com.bloopstudio.mystream.sdPlugin'
OUT = os.path.join(ROOT, 'MyStream.streamDeckPlugin')

with zipfile.ZipFile(OUT, 'w', zipfile.ZIP_DEFLATED) as z:
    for folder, _, files in os.walk(os.path.join(ROOT, NAME)):
        for f in sorted(files):
            path = os.path.join(folder, f)
            z.write(path, os.path.relpath(path, ROOT))
print('Créé :', OUT)
