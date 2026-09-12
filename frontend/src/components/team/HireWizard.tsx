import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { AgentService, KnowledgeService } from "@/services";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Switch } from "@/components/ui/switch";
import { Slider } from "@/components/ui/slider";
import { Chip, KV } from "@/components/common/Bits";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { ArrowLeft, ArrowRight, Check } from "lucide-react";

const STEPS = [
  "Identity",
  "Role",
  "Engine",
  "Responsibilities",
  "Skills",
  "Scope",
  "Runtime",
  "Preview",
];

export function HireWizard({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
}) {
  const [step, setStep] = useState(0);
  const [name, setName] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [description, setDescription] = useState("");
  const [role, setRole] = useState("Compiler Developer");
  const [engine, setEngine] = useState("codex");
  const [caps, setCaps] = useState<string[]>(["implement", "test", "create findings"]);
  const [packs, setPacks] = useState<string[]>(["sk-rust"]);
  const [scopeTeam, setScopeTeam] = useState("Compiler");
  const [paths, setPaths] = useState("/compiler/sema, /tests");
  const [concurrency, setConcurrency] = useState(1);
  const [resourceClass, setResourceClass] = useState("normal");
  const [worktree, setWorktree] = useState(true);

  const { data: roles } = useQuery({ queryKey: ["roles"], queryFn: AgentService.roles });
  const { data: engines } = useQuery({ queryKey: ["engines"], queryFn: AgentService.engines });
  const { data: capabilities } = useQuery({
    queryKey: ["caps"],
    queryFn: AgentService.capabilities,
  });
  const { data: skillPacks } = useQuery({
    queryKey: ["skillPacks"],
    queryFn: KnowledgeService.skillPacks,
  });

  const toggle = (list: string[], setList: (v: string[]) => void, value: string) =>
    setList(list.includes(value) ? list.filter((v) => v !== value) : [...list, value]);

  const finish = () => {
    toast.success(`${displayName || name || "Employee"} hired`, {
      description: `${role} on ${engine} · concurrency ${concurrency}`,
    });
    onOpenChange(false);
    setStep(0);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl gap-0 p-0">
        <DialogHeader className="border-b border-border px-4 py-3">
          <DialogTitle className="text-[14px]">Hire employee</DialogTitle>
          <div className="mt-2 flex flex-wrap gap-1">
            {STEPS.map((s, i) => (
              <button
                key={s}
                onClick={() => setStep(i)}
                className={cn(
                  "rounded px-1.5 py-0.5 text-[11px] transition-colors",
                  i === step
                    ? "bg-primary text-primary-foreground"
                    : i < step
                      ? "bg-secondary text-secondary-foreground"
                      : "text-muted-foreground hover:bg-accent",
                )}
              >
                {i + 1}. {s}
              </button>
            ))}
          </div>
        </DialogHeader>

        <div className="max-h-[52vh] overflow-auto px-4 py-3">
          {step === 0 && (
            <div className="space-y-3">
              <Field label="Name">
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Nina"
                  className="h-8 text-[12.5px]"
                />
              </Field>
              <Field label="Display name">
                <Input
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder="Nina — Parser"
                  className="h-8 text-[12.5px]"
                />
              </Field>
              <Field label="Description">
                <Textarea
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="Focused on parser and lexer work, owns golden diagnostics."
                  className="min-h-20 text-[12.5px]"
                />
              </Field>
            </div>
          )}

          {step === 1 && (
            <Grid>
              {roles?.map((r) => (
                <Selectable key={r} active={role === r} onClick={() => setRole(r)} title={r} />
              ))}
            </Grid>
          )}

          {step === 2 && (
            <Grid>
              {engines?.map((e) => (
                <Selectable
                  key={e.id}
                  active={engine === e.id}
                  onClick={() => setEngine(e.id)}
                  title={e.name}
                  subtitle={e.detail}
                />
              ))}
            </Grid>
          )}

          {step === 3 && (
            <Grid>
              {capabilities?.map((c) => (
                <label
                  key={c}
                  className="flex items-center gap-2 rounded-md border border-border px-2.5 py-2 text-[12.5px] capitalize hover:border-border-strong"
                >
                  <Checkbox
                    checked={caps.includes(c)}
                    onCheckedChange={() => toggle(caps, setCaps, c)}
                  />
                  {c}
                </label>
              ))}
              <p className="col-span-full text-[11px] text-muted-foreground">
                Merge and deploy capabilities are still governed by the pipeline — only the
                integration stage may write to main.
              </p>
            </Grid>
          )}

          {step === 4 && (
            <Grid>
              {skillPacks?.map((p) => (
                <label
                  key={p.id}
                  className="flex items-start gap-2 rounded-md border border-border px-2.5 py-2 hover:border-border-strong"
                >
                  <Checkbox
                    checked={packs.includes(p.id)}
                    onCheckedChange={() => toggle(packs, setPacks, p.id)}
                  />
                  <span>
                    <span className="block text-[12.5px] font-medium">
                      {p.name} <span className="mono-xs text-muted-foreground">v{p.version}</span>
                    </span>
                    <span className="block text-[11px] text-muted-foreground">{p.description}</span>
                  </span>
                </label>
              ))}
            </Grid>
          )}

          {step === 5 && (
            <div className="space-y-3">
              <Field label="Project">
                <Input defaultValue="Surge Compiler" readOnly className="h-8 text-[12.5px]" />
              </Field>
              <Field label="Team">
                <Input
                  value={scopeTeam}
                  onChange={(e) => setScopeTeam(e.target.value)}
                  className="h-8 text-[12.5px]"
                />
              </Field>
              <Field label="Allowed repository paths">
                <Textarea
                  value={paths}
                  onChange={(e) => setPaths(e.target.value)}
                  className="min-h-16 font-mono text-[12px]"
                />
              </Field>
            </div>
          )}

          {step === 6 && (
            <div className="space-y-4">
              <div>
                <div className="flex items-center justify-between text-[12.5px]">
                  <Label>Concurrency</Label>
                  <span className="mono-xs">{concurrency}</span>
                </div>
                <Slider
                  value={[concurrency]}
                  min={1}
                  max={4}
                  step={1}
                  className="mt-2"
                  onValueChange={(v) => setConcurrency(v[0] ?? 1)}
                />
              </div>
              <Field label="Resource class">
                <div className="flex gap-1.5">
                  {["light", "normal", "heavy"].map((c) => (
                    <Button
                      key={c}
                      size="sm"
                      variant={resourceClass === c ? "default" : "outline"}
                      className="h-7 text-[12px] capitalize"
                      onClick={() => setResourceClass(c)}
                    >
                      {c}
                    </Button>
                  ))}
                </div>
              </Field>
              <label className="flex items-center justify-between rounded-md border border-border px-2.5 py-2">
                <span className="text-[12.5px]">Worktree required</span>
                <Switch checked={worktree} onCheckedChange={setWorktree} />
              </label>
            </div>
          )}

          {step === 7 && (
            <div className="panel p-3">
              <p className="section-label mb-2">Employee configuration</p>
              <div className="grid gap-x-6 md:grid-cols-2">
                <KV k="Name" v={displayName || name || "—"} />
                <KV k="Role" v={role} />
                <KV k="Engine" v={engines?.find((e) => e.id === engine)?.name ?? engine} />
                <KV k="Manager" v="Max (Tech Lead)" />
                <KV k="Concurrency" v={String(concurrency)} />
                <KV k="Resource class" v={resourceClass} />
                <KV k="Worktree" v={worktree ? "required" : "shared"} />
                <KV k="Team" v={scopeTeam} />
              </div>
              <p className="section-label mt-3 mb-1">Responsibilities</p>
              <div className="flex flex-wrap gap-1">
                {caps.map((c) => (
                  <Chip key={c} tone="primary">
                    {c}
                  </Chip>
                ))}
              </div>
              <p className="section-label mt-3 mb-1">Skill packs</p>
              <div className="flex flex-wrap gap-1">
                {packs.map((p) => (
                  <Chip key={p} tone="info">
                    {skillPacks?.find((s) => s.id === p)?.name}
                  </Chip>
                ))}
              </div>
              <p className="section-label mt-3 mb-1">Scope</p>
              <p className="mono-xs text-muted-foreground">{paths}</p>
              {description && (
                <>
                  <p className="section-label mt-3 mb-1">Description</p>
                  <p className="text-[12px] text-muted-foreground">{description}</p>
                </>
              )}
            </div>
          )}
        </div>

        <div className="flex items-center justify-between border-t border-border px-4 py-2.5">
          <span className="text-[11.5px] text-muted-foreground">
            Step {step + 1} of {STEPS.length} · {STEPS[step]}
          </span>
          <div className="flex gap-1.5">
            <Button
              size="sm"
              variant="outline"
              className="h-7 text-[12px]"
              disabled={step === 0}
              onClick={() => setStep((s) => s - 1)}
            >
              <ArrowLeft className="size-3.5" /> Back
            </Button>
            {step < STEPS.length - 1 ? (
              <Button size="sm" className="h-7 text-[12px]" onClick={() => setStep((s) => s + 1)}>
                Next <ArrowRight className="size-3.5" />
              </Button>
            ) : (
              <Button size="sm" className="h-7 text-[12px]" onClick={finish}>
                <Check className="size-3.5" /> Hire employee
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-1.5">
      <Label className="text-[11.5px]">{label}</Label>
      {children}
    </div>
  );
}

function Grid({ children }: { children: React.ReactNode }) {
  return <div className="grid gap-1.5 sm:grid-cols-2">{children}</div>;
}

function Selectable({
  active,
  onClick,
  title,
  subtitle,
}: {
  active: boolean;
  onClick: () => void;
  title: string;
  subtitle?: string;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "rounded-md border px-2.5 py-2 text-left transition-colors",
        active ? "border-primary bg-primary/10" : "border-border hover:border-border-strong",
      )}
    >
      <span className="block text-[12.5px] font-medium">{title}</span>
      {subtitle && <span className="block text-[11px] text-muted-foreground">{subtitle}</span>}
    </button>
  );
}
