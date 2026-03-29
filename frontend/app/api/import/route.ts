import { NextRequest, NextResponse } from 'next/server';
import { db, experts, workflows, workflowSteps, datasets, datasetSchemas, lineage, plans } from '@/lib/db';
import { nanoid } from '../tasks/nanoid';

/* POST /api/import — import a Kortecx export config */
export async function POST(req: NextRequest) {
  const body = await req.json();

  if (body._kortecxExport !== true) {
    return NextResponse.json({ error: 'Invalid file: not a Kortecx export' }, { status: 400 });
  }

  const entityType = body._entityType;
  if (!entityType) {
    return NextResponse.json({ error: 'Missing _entityType' }, { status: 400 });
  }

  try {
    switch (entityType) {
      case 'expert': {
        const src = body.expert;
        if (!src) return NextResponse.json({ error: 'Missing expert data' }, { status: 400 });

        const newId = `exp-${nanoid()}`;
        await db.insert(experts).values({
          id: newId,
          name: src.name || 'Imported Expert',
          description: src.description || null,
          role: src.role || 'assistant',
          status: 'idle',
          version: src.version || '1.0.0',
          modelId: src.modelId || '',
          modelName: src.modelName || null,
          providerId: src.providerId || '',
          providerName: src.providerName || null,
          modelSource: src.modelSource || 'provider',
          localModelConfig: src.localModelConfig || null,
          systemPrompt: src.systemPrompt || null,
          temperature: src.temperature != null ? String(src.temperature) : '0.7',
          maxTokens: src.maxTokens || 4096,
          tags: src.tags || [],
          isPublic: src.isPublic || false,
          isFinetuned: src.isFinetuned || false,
          category: src.category || 'custom',
          complexityLevel: src.complexityLevel || 3,
        });

        // Lineage record
        await db.insert(lineage).values({
          id: `lin-${nanoid()}`,
          sourceType: 'import',
          sourceId: body._exportedAt || 'unknown',
          targetType: 'expert',
          targetId: newId,
          relationship: 'created_by',
          metadata: { importedFrom: body._exportedAt, originalId: src.id },
        });

        return NextResponse.json({ id: newId, name: src.name, entityType: 'expert' });
      }

      case 'workflow': {
        const src = body.workflow;
        if (!src) return NextResponse.json({ error: 'Missing workflow data' }, { status: 400 });

        const newId = `wf-${nanoid()}`;

        // Import plan if present
        let newPlanId: string | null = null;
        if (body.plan) {
          newPlanId = `plan-${nanoid()}`;
          await db.insert(plans).values({
            id: newPlanId,
            workflowId: newId,
            name: body.plan.name || `${src.name} Plan`,
            description: body.plan.description || null,
            dag: body.plan.dag || null,
            status: 'draft',
            generatedBy: 'import',
            version: 1,
            planType: body.plan.planType || 'live',
            markdownContent: body.plan.markdownContent || null,
            sourceType: 'upload',
          });
        }

        await db.insert(workflows).values({
          id: newId,
          name: src.name || 'Imported Workflow',
          description: src.description || null,
          goalStatement: src.goalStatement || null,
          status: 'draft',
          tags: src.tags || [],
          isTemplate: src.isTemplate || false,
          templateCategory: src.templateCategory || null,
          metadata: src.metadata || null,
          activePlanId: newPlanId,
        });

        // Import steps
        const steps = body.steps || [];
        if (steps.length > 0) {
          await db.insert(workflowSteps).values(
            steps.map((s: Record<string, unknown>, i: number) => ({
              id: `ws-${nanoid()}`,
              workflowId: newId,
              order: i + 1,
              name: (s.name as string) || null,
              description: (s.description as string) || null,
              expertId: (s.expertId as string) || null,
              taskDescription: (s.taskDescription as string) || '',
              systemInstructions: (s.systemInstructions as string) || null,
              voiceCommand: (s.voiceCommand as string) || null,
              fileLocations: (s.fileLocations as string[]) || [],
              stepFileUrls: (s.stepFileUrls as string[]) || [],
              stepImageUrls: (s.stepImageUrls as string[]) || [],
              integrations: s.integrations || null,
              modelSource: (s.modelSource as string) || 'provider',
              localModelConfig: s.localModelConfig || null,
              connectionType: (s.connectionType as string) || 'sequential',
              stepType: (s.stepType as string) || 'agent',
              actionConfig: s.actionConfig || null,
              shareMemory: s.shareMemory !== false,
              temperature: s.temperature != null ? String(s.temperature) : '0.7',
              maxTokens: (s.maxTokens as number) || 4096,
            })),
          );
        }

        // Lineage record
        await db.insert(lineage).values({
          id: `lin-${nanoid()}`,
          sourceType: 'import',
          sourceId: body._exportedAt || 'unknown',
          targetType: 'workflow',
          targetId: newId,
          relationship: 'created_by',
          metadata: { importedFrom: body._exportedAt, originalId: src.id },
        });

        return NextResponse.json({ id: newId, name: src.name, entityType: 'workflow' });
      }

      case 'dataset': {
        const src = body.dataset;
        if (!src) return NextResponse.json({ error: 'Missing dataset data' }, { status: 400 });

        const newId = `ds-${nanoid()}`;
        await db.insert(datasets).values({
          id: newId,
          name: src.name || 'Imported Dataset',
          description: src.description || null,
          status: 'draft',
          format: src.format || 'jsonl',
          sampleCount: src.sampleCount || 0,
          tags: src.tags || [],
          categories: src.categories || [],
        });

        // Import schema columns if present
        const schemaCols = body.schema || [];
        if (schemaCols.length > 0) {
          await db.insert(datasetSchemas).values(
            schemaCols.map((col: Record<string, unknown>) => ({
              id: `dsc-${nanoid()}`,
              datasetId: newId,
              columnName: (col.columnName as string) || '',
              dataType: (col.dataType as string) || 'string',
              nullable: col.nullable !== false,
              description: (col.description as string) || null,
              sampleValues: col.sampleValues || null,
            })),
          );
        }

        // Lineage record
        await db.insert(lineage).values({
          id: `lin-${nanoid()}`,
          sourceType: 'import',
          sourceId: body._exportedAt || 'unknown',
          targetType: 'dataset',
          targetId: newId,
          relationship: 'created_by',
          metadata: { importedFrom: body._exportedAt, originalId: src.id },
        });

        return NextResponse.json({ id: newId, name: src.name, entityType: 'dataset' });
      }

      default:
        return NextResponse.json({ error: `Unsupported import type: ${entityType}` }, { status: 400 });
    }
  } catch (err) {
    console.error('Import error:', err);
    return NextResponse.json({ error: 'Import failed' }, { status: 500 });
  }
}
