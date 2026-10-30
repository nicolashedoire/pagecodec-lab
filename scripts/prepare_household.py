#!/usr/bin/env python3
"""Download and exactly convert the full UCI household-power numeric corpus."""
import argparse
import array
import datetime
import hashlib
import json
import struct
import subprocess
import sys
import zipfile
from decimal import Decimal
from pathlib import Path

URL = 'https://archive.ics.uci.edu/static/public/235/individual%2Bhousehold%2Belectric%2Bpower%2Bconsumption.zip'
ARCHIVE_SHA256 = '9f84b46ade8a2d8e1286ec4b2b6c2987a45a755c59f263be3b3b3d10dfbda3ff'
MEMBER = 'household_power_consumption.txt'
EXPECTED_ROWS = 2075259
MAX_CHUNK = 64 * 1024 * 1024
MISSING = -(2**31)
FIELDS = ['Global_active_power', 'Global_reactive_power', 'Voltage', 'Global_intensity',
          'Sub_metering_1', 'Sub_metering_2', 'Sub_metering_3']
# Fixed before codec measurements, checked exactly for every non-missing value.
SCALES = [1000, 1000, 100, 10, 1, 1, 1]


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as handle:
        for block in iter(lambda: handle.read(1024*1024), b''):
            digest.update(block)
    return digest.hexdigest()


def build(root, archive):
    target = root/'data/household'
    target.mkdir(parents=True, exist_ok=True)
    if sha256(archive) != ARCHIVE_SHA256:
        raise RuntimeError('Archive SHA-256 does not match the pinned source')
    columns = [array.array('i') for _ in range(8)]
    if columns[0].itemsize != 4:
        raise RuntimeError('This converter requires a 32-bit array int type')
    counts = [0]*7
    source_digest = hashlib.sha256()
    source_size = 0
    chunk_files = []
    chunk = None
    chunk_bytes = 0
    rows = 0
    origin = None
    last_timestamp = None
    row_path = target/'household-row-i32.bin'
    with zipfile.ZipFile(archive) as z, z.open(MEMBER) as source, row_path.open('wb') as output:
        if z.getinfo(MEMBER).file_size != 132960755:
            raise RuntimeError('Unexpected source text size')
        for line_index, line in enumerate(source):
            source_digest.update(line)
            source_size += len(line)
            if chunk is None or chunk_bytes + len(line) > MAX_CHUNK:
                if chunk is not None:
                    chunk.close()
                name = f'household-text-part-{len(chunk_files)+1:02}.txt'
                chunk_files.append(name)
                chunk = (target/name).open('wb')
                chunk_bytes = 0
            chunk.write(line)
            chunk_bytes += len(line)
            values = line.decode('ascii').rstrip('\r\n').split(';')
            if line_index == 0:
                if values != ['Date','Time']+FIELDS:
                    raise RuntimeError('Unexpected source header')
                continue
            if len(values) != 9:
                raise RuntimeError(f'Unexpected field count at row {rows}')
            day, month, year = map(int, values[0].split('/'))
            hour, minute, second = map(int, values[1].split(':'))
            timestamp = datetime.datetime(year, month, day, hour, minute, second)
            if second != 0:
                raise RuntimeError('Sub-minute timestamp cannot be encoded exactly')
            if origin is None:
                origin = timestamp
            elapsed = int((timestamp-origin).total_seconds())
            if elapsed % 60:
                raise RuntimeError('Non-integral minute offset')
            # Preserve the source's naive calendar timestamps, without inventing a timezone.
            row = [elapsed//60]
            for i, (value, scale) in enumerate(zip(values[2:], SCALES)):
                if value in ('', '?'):
                    integer = MISSING
                    counts[i] += 1
                else:
                    decimal = Decimal(value)
                    scaled = decimal * scale
                    if not scaled.is_finite() or scaled != scaled.to_integral_value():
                        raise RuntimeError(f'Fixed scale is not exact at row {rows}, field {FIELDS[i]}')
                    integer = int(scaled)
                    if not MISSING < integer < 2**31:
                        raise RuntimeError('Integer range or missing-sentinel collision')
                    if Decimal(integer)/scale != decimal:
                        raise RuntimeError('Numeric round trip failed')
                row.append(integer)
            output.write(struct.pack('<8i', *row))
            for column, value in zip(columns, row):
                column.append(value)
            rows += 1
            last_timestamp = timestamp
            if rows % 500000 == 0:
                print(f'Converted {rows:,} original rows...', flush=True)
    if chunk is not None:
        chunk.close()
    if rows != EXPECTED_ROWS:
        raise RuntimeError(f'Expected {EXPECTED_ROWS} rows, got {rows}')
    col_path = target/'household-column-i32.bin'
    with col_path.open('wb') as output:
        for column in columns:
            if sys.byteorder != 'little':
                column.byteswap()
            column.tofile(output)
    # Verify the text chunks concatenate to the exact original byte stream.
    reconstruction = hashlib.sha256()
    for name in chunk_files:
        with (target/name).open('rb') as handle:
            for block in iter(lambda: handle.read(1024*1024), b''):
                reconstruction.update(block)
    if reconstruction.hexdigest() != source_digest.hexdigest():
        raise RuntimeError('Text partition did not preserve the original bytes')
    items = []
    for name, kind in [(row_path.name,'row-major fixed-point int32'),
                       (col_path.name,'column-major fixed-point int32')]+[(n,'original text chunk') for n in chunk_files]:
        path = target/name
        if path.stat().st_size > MAX_CHUNK:
            raise RuntimeError('Benchmark input exceeds 64 MiB limit')
        items.append({'name':name,'path':'data/household/'+name,'size':path.stat().st_size,
                      'sha256':sha256(path),'category':'real-household-power',
                      'representation':kind,'source_corpus':'uci-household-power-235'})
    return {'schema_version':1,'source':{'name':'Individual Household Electric Power Consumption',
        'authors':['Georges Hebrail','Alice Berard'],'doi':'https://doi.org/10.24432/C58K54',
        'url':URL,'license':'CC-BY-4.0','license_url':'https://creativecommons.org/licenses/by/4.0/',
        'archive_sha256':ARCHIVE_SHA256,'source_sha256':source_digest.hexdigest(),'source_bytes':source_size},
        'conversion':{'rows':rows,'first_timestamp':origin.isoformat(),'last_timestamp':last_timestamp.isoformat(),
            'fields':['relative_calendar_minutes']+FIELDS,'scales':[1]+SCALES,'byte_order':'little',
            'value_type':'signed int32','missing_sentinel':MISSING,'missing_per_field':dict(zip(FIELDS,counts)),
            'losslessness':'Exact numeric values, timestamp values and missingness; not original text spelling. Text chunks preserve original bytes.',
            'filtering':'None. Every original row retained once in each representation. No repeated padding.',
            'layout':'Row: eight values per record. Column: eight complete contiguous arrays, same values and order within each array.',
            'independence':'Four benchmark files are representations/partitions of ONE source corpus, not four independent datasets.'},
        'protocol':{'page_bytes':16384,'repetitions':9,'warmups':1,'codec':'frozen V2',
            'strict_win':'size <= reference AND encode <=0.90*reference AND decode <=0.90*reference'},
        'datasets':items}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive',type=Path,help='Use an already downloaded official archive')
    parser.add_argument('--record-manifest',action='store_true',help='Maintainer: create the pinned manifest before measuring')
    args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]
    archive=args.archive or root/'data/downloads/household-power.zip'
    if not archive.exists():
        archive.parent.mkdir(parents=True,exist_ok=True)
        subprocess.run(['curl','-fL','--retry','2','--max-time','180',URL,'-o',str(archive)],check=True)
    actual=build(root,archive)
    pinned=root/'manifests/household.json'
    if args.record_manifest:
        pinned.parent.mkdir(parents=True,exist_ok=True)
        pinned.write_text(json.dumps(actual,indent=2)+'\n')
    else:
        expected=json.loads(pinned.read_text())
        if actual != expected:
            raise RuntimeError('Converted corpus differs from the published manifest')
    print(f"Verified all {actual['conversion']['rows']:,} rows; "+', '.join(f"{d['name']}: {d['size']/2**20:.2f} MiB" for d in actual['datasets']))


if __name__=='__main__':
    main()
