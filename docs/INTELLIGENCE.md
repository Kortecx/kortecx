# Intelligence — Fine-tuning, Inference & Models

## Overview

The Intelligence section provides model lifecycle management: fine-tuning local models, managing model registries, and accessing cloud inference. Local features are fully functional; cloud features redirect to [Kortecx Cloud](https://www.kortecx.com).

---

## Fine-tuning (`/intelligence/finetuning`)

**Status:** Fully functional (local)

Create LoRA fine-tuning jobs for local models using custom datasets.

### Features
- **Job creation** — name, base model (from installed Ollama/llama.cpp models), dataset path, epochs, learning rate, batch size
- **Target expert** — optionally link the adapter to an existing expert
- **Job tracking** — status badges, progress bars, epoch counters
- **Stats dashboard** — total/running/completed/failed job counts
- **Persistence** — jobs saved to localStorage, creation logged to system logs

### Supported Configuration

| Parameter | Default | Range |
|-----------|---------|-------|
| Epochs | 3 | 1–50 |
| Learning Rate | 0.0002 | 0.00001–1.0 |
| Batch Size | 4 | 1–64 |
| Engine | Ollama | Ollama, llama.cpp |

---

## Inference (`/intelligence/inference`)

**Status:** Cloud-only (disabled locally)

Managed inference endpoints are available exclusively on Kortecx Cloud. The page displays feature previews and directs users to sign up.

### Cloud Features (preview)
- Auto-scaling (0 to thousands of req/sec)
- Global edge deployment (30+ regions)
- Dedicated GPUs (A100, H100, L40S)
- Enterprise SLA (99.9% uptime)
- SOC 2 / HIPAA compliance
- Model registry with versioning

### Local Alternative
For local inference, use:
- **Workflow Builder** — each step can target Ollama or llama.cpp
- **Settings → Inference** — configure local backend URLs and defaults

---

## Models (`/intelligence/models`)

**Status:** Local tab functional, cloud tabs redirect

### Three Tabs

| Tab | Status | Description |
|-----|--------|-------------|
| **Local Models** | Enabled | Full model management for Ollama/llama.cpp |
| **Kortecx Models** | Cloud | Redirects to https://www.kortecx.com |
| **Advanced Models** | Cloud | Redirects to https://www.kortecx.com |

### Local Models Features
- **Engine toggle** — switch between Ollama and llama.cpp
- **Model list** — name, size, last modified
- **Search** — filter installed models
- **Pull** — download models from registry with SSE streaming progress bar
- **Delete** — remove models from local storage
- **Refresh** — re-fetch model list from engine

### Kortecx Models (Cloud)
Domain-specific models fine-tuned by the Kortecx team for coding, research, legal, finance, and more. Available with a Kortecx Cloud subscription.

### Advanced Models (Cloud)
Enterprise-grade models with extended context, multi-modal capabilities, and custom training. Includes GPT-4o, Claude Opus, Gemini Ultra, and Kortecx MoE models.

---

## Timezone Handling

All timestamps across the platform use a static list of 75 globally-referenced IANA timezones defined in `lib/timezones.ts`. This avoids SSR hydration mismatches caused by `Intl.supportedValuesOf('timeZone')` returning different results on server vs client.

### Coverage
- UTC
- 10 Americas (New York, Chicago, Denver, LA, Toronto, Vancouver, Mexico City, São Paulo, Buenos Aires, Santiago)
- 19 Europe (London, Paris, Berlin, Madrid, Rome, Amsterdam, Zurich, Stockholm, Moscow, Istanbul, etc.)
- 5 Africa (Cairo, Lagos, Johannesburg, Nairobi, Casablanca)
- 3 Middle East (Dubai, Riyadh, Tehran)
- 4 South Asia (Karachi, Kolkata, Colombo, Dhaka)
- 10 East/Southeast Asia (Bangkok, Singapore, Shanghai, Hong Kong, Tokyo, Seoul, etc.)
- 7 Oceania (Sydney, Melbourne, Brisbane, Perth, Auckland, Fiji, Honolulu)

### Usage
```typescript
import { TIMEZONES, formatTzLabel } from '@/lib/timezones';

// In select dropdown
{TIMEZONES.map(tz => (
  <option key={tz} value={tz}>{formatTzLabel(tz)}</option>
))}

// Format timestamp in specific timezone
new Date(iso).toLocaleTimeString('en-US', { timeZone: selectedTz });
```
