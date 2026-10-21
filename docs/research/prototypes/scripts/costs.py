#!/usr/bin/env python3
"""Worked monthly cost example for Kairo cloud sharing.
Scenario: 1,000 users x 50 clips/month x 30 MB, 30-day retention, 20 views per clip.
Prices verified 2026-10-21 (see report Sources). 1 GB = 1,000 MB, 1 TB = 1,000 GB (approximation;
AWS/R2 bill binary GB, which lowers the totals by ~7%)."""

users, clips_per_user, mb_per_clip, views_per_clip = 1000, 50, 30, 20
clip_seconds = 30          # assumption: 30 MB ~ 30 s at ~8 Mbit/s (1080p60 H.264 share copy)
clips = users * clips_per_user
upload_gb = clips * mb_per_clip / 1000
stored_gb = upload_gb       # 30-day retention -> steady state ~ one month of uploads
views = clips * views_per_clip
egress_gb = views * mb_per_clip / 1000   # worst case: every view downloads the whole file
stored_min = clips * clip_seconds / 60
delivered_min = views * clip_seconds / 60

print(f"clips/month={clips:,}  uploaded={upload_gb:,.0f} GB  stored(steady)={stored_gb:,.0f} GB")
print(f"views/month={views:,}  egress(worst case)={egress_gb:,.0f} GB  stored_min={stored_min:,.0f}  delivered_min={delivered_min:,.0f}")
print()

rows = []

# Cloudflare R2 (Standard): $0.015/GB-mo (10 GB free), Class A $4.50/M (1M free), Class B $0.36/M (10M free), egress $0
class_a = clips * 2                      # 1 PUT for clip + 1 for thumbnail/sprite
for reqs_per_view, label in [(3, "3 range req/view, no CDN cache"), (1, "mostly CDN cache hits")]:
    class_b = views * reqs_per_view + views  # + poster image fetch
    r2 = max(stored_gb - 10, 0) * 0.015 + max(class_a - 1e6, 0) / 1e6 * 4.50 + max(class_b - 10e6, 0) / 1e6 * 0.36
    rows.append((f"Cloudflare R2 ({label})", r2))

# Cloudflare Stream: $5 per 1,000 min stored (prepaid, bought in 1,000-min blocks), $1 per 1,000 min delivered
import math
stream = math.ceil(stored_min / 1000) * 5 + delivered_min / 1000 * 1
rows.append(("Cloudflare Stream", stream))

# Bunny Stream: storage $0.01/GB (1 region; billed on everything encoding generates), CDN Volume $0.005/GB or Standard EU/NA $0.01/GB
for stor_mult, cdn_rate, label in [(1.0, 0.005, "MP4 only, Volume tier"), (1.6, 0.010, "renditions +60%, Standard EU/NA")]:
    bunny = stored_gb * stor_mult * 0.01 + egress_gb * cdn_rate
    rows.append((f"Bunny Stream ({label})", max(bunny, 1)))

# Mux pay-as-you-go, basic quality: input free; storage $0.0024/min (<=720p) or $0.003/min (1080p);
# delivery first 100k min free, then $0.0008 (<=720p) / $0.0010 (1080p) per min in the first 500k tier; $20 monthly credit
for res, s_rate, d_rate in [("720p", 0.0024, 0.0008), ("1080p", 0.0030, 0.0010)]:
    billable = max(delivered_min - 100_000, 0)
    first = min(billable, 500_000)
    mux = stored_min * s_rate + first * d_rate
    rows.append((f"Mux basic {res} (before $20 PAYG credit)", mux))

# Backblaze B2: $6.95/TB-month (first 10 GB free); free egress up to 3x stored, then $0.01/GB
b2_storage = max(stored_gb - 10, 0) / 1000 * 6.95
b2_direct_egress = max(egress_gb - 3 * stored_gb, 0) * 0.01
rows.append(("Backblaze B2, served directly", b2_storage + b2_direct_egress))
rows.append(("Backblaze B2 + Bunny CDN Volume ($0.005/GB, B2->Bunny egress free)", b2_storage + egress_gb * 0.005))

# AWS S3 Standard us-east-1 $0.023/GB-mo, PUT $0.005/1k; CloudFront PAYG: first 1 TB free, next 9 TB $0.085, next 40 TB $0.080
s3 = stored_gb * 0.023 + class_a / 1000 * 0.005
cf_gb = egress_gb
tiers = [(1000, 0.0), (9000, 0.085), (40000, 0.080), (100000, 0.060)]
cf = 0.0
remaining = cf_gb
for size, rate in tiers:
    take = min(remaining, size)
    cf += take * rate
    remaining -= take
    if remaining <= 0:
        break
requests = views * 3 + views
cf += max(requests - 10e6, 0) / 10000 * 0.0100
rows.append(("AWS S3 + CloudFront pay-as-you-go", s3 + cf))
# CloudFront flat-rate Pro: $15/mo, 50 TB + 10M requests allowance, 50 GB S3 credit
rows.append(("AWS S3 + CloudFront flat-rate Pro ($15)", 15 + s3 - 50 * 0.023))

for name, cost in rows:
    print(f"{name:70s} ${cost:>9,.2f}/month")

print()
print("Discord target-size bitrates (95% of 20 MB decimal = 19,000,000 bytes; 160 kbit/s audio):")
for limit_mb, label in [(20, "Discord free (20 MB)"), (50, "Nitro Basic / Boost L2 (50 MB)"), (100, "Boost L3 (100 MB)")]:
    budget_bits = limit_mb * 1_000_000 * 0.95 * 8
    parts = []
    for dur in (15, 30, 60, 120):
        video_kbps = budget_bits / dur / 1000 - 160
        parts.append(f"{dur:>3}s: {video_kbps:>7,.0f} kb/s")
    print(f"  {label:32s} " + " | ".join(parts))
