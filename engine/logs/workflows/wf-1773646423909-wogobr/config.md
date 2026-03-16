---
workflow: wf-1773646423909-wogobr
saved_at: 2026-03-16T07:34:35.697213Z
---

# Workflow Configuration

## workflow

- **name**: test workflow
- **stepsCount**: 1

## metrics

- **mlflow**: False
- **mlflowTrackingUri**: http://localhost:5000
- **mlflowExperiment**: kortecx-workflows
- **logging**: True
- **logLevel**: info
- **logFormat**: structured
- **monitoring**: False
- **monitoringInterval**: 30
- **alertOnFailure**: True
- **alertOnLatency**: False
- **latencyThresholdMs**: 5000

## advanced

- **maxRetries**: 2
- **timeoutSec**: 300
- **failureStrategy**: stop
- **priority**: normal
- **concurrencyLimit**: 5
- **cacheResults**: False
- **cacheTtlSec**: 3600
- **notifyOnComplete**: False
- **notifyChannel**: 
- **description**: 

## permissions

- **visibility**: private
- **allowClone**: True
- **allowEdit**: owner
- **requireApproval**: False
- **maxRunsPerDay**: 0
- **tokenBudget**: 0

## tags

- Test tag
