# Kortecx Frontend

Next.js 16 application providing the UI for the Kortecx platform — dashboard, workflow builder, expert management, data synthesis, monitoring, and analytics.

---

## Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| **Node.js** | 20+ | Runtime for Next.js |
| **Docker services** | Running | PostgreSQL must be available (via `docker compose up -d` from the root) |

The engine (FastAPI) should also be running for full functionality. See the [root README](../README.md) for full-stack setup.

---

## Setup

```bash
# From the frontend/ directory
npm install
```

### Environment

The frontend reads environment variables from the root `.env` file. Key variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgresql://kortecx:kortecx@localhost:5433/kortecx_dev` | PostgreSQL connection (Drizzle ORM) |
| `DB_MODE` | `local` | Set to empty for Neon cloud |
| `QDRANT_URL` | `http://localhost:6333` | Qdrant vector database |
| `NEXT_PUBLIC_ENGINE_URL` | `http://localhost:8000` | Engine API base URL (used in browser) |

### Start

```bash
npm run dev:next    # Next.js dev server on http://localhost:3000
```

Or from the root: `make frontend`

---

## Database

The frontend uses **Drizzle ORM** with PostgreSQL. Schema is defined in `lib/db/schema.ts`.

| Command | Description |
|---------|-------------|
| `npm run db:generate` | Generate migration files from schema changes |
| `npm run db:migrate` | Run pending migrations |
| `npm run db:push` | Push schema directly to database (dev) |
| `npm run db:studio` | Open Drizzle Studio visual editor |
| `npm run db:seed` | Populate sample data |

Migration files live in `drizzle/`. Manual SQL migrations (e.g., `0014_action_steps.sql`) are applied directly.

---

## Key Directories

```
frontend/
├── app/                    # Next.js App Router pages and API routes
│   ├── dashboard/          # Main dashboard
│   ├── workflow/            # Workflow builder, listing, history
│   │   └── builder/        # Visual workflow builder (Monaco editors)
│   ├── experts/            # Expert management and marketplace
│   ├── data/               # Data synthesis, assets, datasets
│   ├── training/           # Model fine-tuning UI
│   ├── analytics/          # Usage analytics and metrics
│   ├── monitoring/         # Real-time system monitoring
│   ├── providers/          # AI provider configuration
│   └── api/                # API routes (workflows, assets, experts, data, etc.)
├── lib/                    # Shared code
│   ├── db/                 # Drizzle ORM schema and connection
│   │   ├── schema.ts       # All database table definitions
│   │   └── index.ts        # Database client singleton
│   ├── hooks/              # React hooks (useApi, useDraftCache, useWorkflowLogger)
│   ├── types.ts            # TypeScript type definitions
│   └── constants.ts        # Role metadata, integration catalog, plugins
├── drizzle/                # Database migrations (SQL files)
├── public/                 # Static assets
├── scripts/                # Setup and migration scripts
└── tests/                  # Vitest test files
```

---

## Scripts

| Command | Description |
|---------|-------------|
| `npm run dev` | Full setup script (setup-local.sh) |
| `npm run dev:next` | Next.js dev server |
| `npm run build` | Production build |
| `npm run start` | Start production server |
| `npm run lint` | ESLint (errors only) |
| `npm run lint:all` | ESLint (all warnings) |
| `npm run typecheck` | TypeScript type checking (`tsc --noEmit`) |
| `npm run test` | Run tests once (Vitest) |
| `npm run test:watch` | Test watch mode |
| `npm run test:coverage` | Coverage report |
| `npm run check` | Quick quality gate (lint + typecheck) |
| `npm run check:full` | Full quality gate (lint, test, build) |

---

## Testing

```bash
npm run test              # Run all tests
npm run test:watch        # Watch mode
npm run test:coverage     # With coverage report
```

Uses **Vitest** with jsdom and **@testing-library/react** for component tests.

---

## Tech Stack

| Library | Version | Purpose |
|---------|---------|---------|
| Next.js | 16.1.6 | React framework with App Router |
| React | 19.2.3 | UI library |
| Drizzle ORM | 0.45.1 | Type-safe database ORM |
| SWR | 2.4.1 | Data fetching and caching |
| Monaco Editor | 4.7.0 | Code/markdown editors |
| Tailwind CSS | 4.x | Utility-first styling |
| Lucide React | 0.577+ | Icon library |
| Framer Motion | 12.36+ | Animations |
| TypeScript | 5.x | Type safety |
