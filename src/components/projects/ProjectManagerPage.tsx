import { useMemo, useState } from "react";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Check,
  ChevronDown,
  FolderOpen,
  Plus,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import { projectsApi, providersApi } from "@/lib/api";
import type { ProjectDto, ProjectProviderRoute } from "@/lib/api/projects";
import type { AppId } from "@/lib/api/types";
import type { Provider } from "@/types";
import { getBaseName } from "@/components/sessions/utils";
import { ProviderIcon } from "@/components/ProviderIcon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverAnchor,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { extractErrorMessage } from "@/utils/errorUtils";

const PROJECT_APPS: Array<{ id: AppId; label: string; icon: string }> = [
  { id: "codex", label: "Codex", icon: "openai" },
  { id: "claude", label: "Claude Code", icon: "claude" },
];

interface ForceModelDropdownProps {
  value: string;
  enabled: boolean;
  models: string[];
  selectionDisabled: boolean;
  onDisable: () => void;
  onSelect: (model: string) => void;
  onAdd: (model: string) => Promise<void>;
  onDelete: (model: string) => Promise<void>;
}

function ForceModelDropdown({
  value,
  enabled,
  models,
  selectionDisabled,
  onDisable,
  onSelect,
  onAdd,
  onDelete,
}: ForceModelDropdownProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [pending, setPending] = useState(false);
  const allModels =
    value && !models.includes(value) ? [value, ...models] : models;
  const normalizedSearch = search.trim().toLowerCase();
  const visibleModels = normalizedSearch
    ? allModels.filter((model) =>
        model.toLowerCase().includes(normalizedSearch),
      )
    : allModels;
  const addModel = search.trim();
  const canAddModel = Boolean(addModel) && visibleModels.length === 0;
  const triggerText =
    enabled && value
      ? value
      : value
        ? t("projectManager.forceModelOffWithValue", {
            defaultValue: "已关闭 · {{model}}",
            model: value,
          })
        : t("projectManager.forceModelPlaceholder", {
            defaultValue: "选择模型",
          });

  const closePicker = () => {
    setOpen(false);
    setSearch("");
  };

  const handleSelect = (model: string) => {
    onSelect(model);
    closePicker();
  };

  const handleAdd = async (model: string) => {
    if (!model || pending) return;
    setPending(true);
    try {
      await onAdd(model);
    } catch {
      // The parent handler reports the error.
      return;
    } finally {
      setPending(false);
    }

    onSelect(model);
    closePicker();
  };

  const handleDelete = async (model: string) => {
    if (pending) return;
    setPending(true);
    try {
      await onDelete(model);
    } catch {
      // The parent handler reports the error.
    } finally {
      setPending(false);
    }
  };

  return (
    <Popover
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (!nextOpen) setSearch("");
      }}
    >
      <PopoverAnchor asChild>
        <div className="relative w-[220px] shrink-0">
          <Input
            value={search}
            onFocus={() => {
              setSearch("");
              setOpen(true);
            }}
            onChange={(event) => {
              setSearch(event.target.value);
              setOpen(true);
            }}
            placeholder={triggerText}
            autoComplete="off"
            className="h-9 w-full pr-8 text-sm"
            title={t("projectManager.forceModelSelect", {
              defaultValue: "选择模型",
            })}
          />
          <ChevronDown className="pointer-events-none absolute right-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        </div>
      </PopoverAnchor>
      <PopoverContent
        align="end"
        className="w-[320px] p-2"
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <div className="max-h-64 space-y-0.5 overflow-y-auto">
          <button
            type="button"
            disabled={selectionDisabled}
            onClick={() => {
              onDisable();
              closePicker();
            }}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-muted disabled:cursor-not-allowed disabled:opacity-60"
          >
            <Check
              className={cn(
                "size-3.5 shrink-0 text-emerald-500",
                enabled && "opacity-0",
              )}
            />
            <span className="min-w-0 flex-1 truncate">
              {t("projectManager.forceModelDisable", {
                defaultValue: "关闭强制路由模型",
              })}
            </span>
          </button>

          <div className="my-1 border-t border-border/60" />

          {visibleModels.length === 0 && (
            <div className="px-2 py-5 text-center">
              <div className="text-xs text-muted-foreground">
                {t("projectManager.forceModelEmpty", {
                  defaultValue: "暂无匹配模型",
                })}
              </div>
              {canAddModel && (
                <button
                  type="button"
                  disabled={selectionDisabled || pending}
                  onClick={() => void handleAdd(addModel)}
                  className="mt-2 inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs text-primary transition-colors hover:bg-muted disabled:cursor-not-allowed disabled:opacity-60"
                >
                  <Plus className="size-3.5 shrink-0" />
                  <span className="min-w-0 truncate">
                    {t("projectManager.forceModelAddNamed", {
                      defaultValue: "新增“{{model}}”",
                      model: addModel,
                    })}
                  </span>
                </button>
              )}
            </div>
          )}
          {visibleModels.map((model) => {
            const selected = enabled && model === value;
            const stale = !models.includes(model);
            const lastUsed = !enabled && model === value;
            return (
              <div
                key={model}
                className="group flex items-center gap-1 rounded-md hover:bg-muted"
              >
                <button
                  type="button"
                  disabled={selectionDisabled}
                  onClick={() => handleSelect(model)}
                  className="flex min-w-0 flex-1 items-center gap-2 px-2 py-1.5 text-left text-xs disabled:cursor-not-allowed disabled:opacity-60"
                >
                  <Check
                    className={cn(
                      "size-3.5 shrink-0 text-emerald-500",
                      !selected && "opacity-0",
                    )}
                  />
                  <span className="min-w-0 flex-1 truncate">{model}</span>
                  {lastUsed && (
                    <span className="shrink-0 text-[9px] text-muted-foreground">
                      {t("projectManager.forceModelLastUsed", {
                        defaultValue: "上次使用",
                      })}
                    </span>
                  )}
                  {stale && (
                    <span className="shrink-0 text-[9px] text-amber-500">
                      {t("projectManager.forceModelRemoved", {
                        defaultValue: "已移除",
                      })}
                    </span>
                  )}
                </button>
                {!stale && (
                  <button
                    type="button"
                    disabled={pending}
                    onClick={() => void handleDelete(model)}
                    className="mr-1 rounded p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100 disabled:opacity-40"
                    title={t("projectManager.forceModelDelete", {
                      defaultValue: "从全局列表删除",
                    })}
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                )}
              </div>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
export function ProjectManagerPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [activeProjectApp, setActiveProjectApp] = useState<AppId>("codex");

  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => projectsApi.list(),
  });

  const forceModels = useQuery({
    queryKey: ["project-force-models"],
    queryFn: () => projectsApi.listForceModels(),
    staleTime: Infinity,
  });

  const providerQueries = useQueries({
    queries: PROJECT_APPS.map((app) => ({
      queryKey: ["project-providers", app.id],
      queryFn: () => providersApi.getAll(app.id),
      staleTime: 60_000,
    })),
  });

  const providerMaps = useMemo(
    () =>
      PROJECT_APPS.reduce<Record<string, Record<string, Provider>>>(
        (result, app, index) => {
          result[app.id] = providerQueries[index]?.data ?? {};
          return result;
        },
        {},
      ),
    [providerQueries],
  );

  const filteredProjects = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (projects.data ?? []).filter((project) => {
      const appSessions = project.sessions.filter(
        (session) => session.providerId === activeProjectApp,
      );
      if (appSessions.length === 0) return false;
      if (!query) return true;

      return (
        project.projectPath.toLowerCase().includes(query) ||
        getBaseName(project.projectPath).toLowerCase().includes(query) ||
        appSessions.some((session) =>
          (session.title || session.summary || "")
            .toLowerCase()
            .includes(query),
        )
      );
    });
  }, [activeProjectApp, projects.data, search]);

  const setRouteInCache = (
    projectPath: string,
    appType: AppId,
    route: ProjectProviderRoute,
  ) => {
    queryClient.setQueryData<ProjectDto[]>(["projects"], (current) =>
      current?.map((project) =>
        project.projectPath === projectPath
          ? {
              ...project,
              routes: [
                ...project.routes.filter((item) => item.appType !== appType),
                route,
              ],
            }
          : project,
      ),
    );
  };

  const removeRouteFromCache = (projectPath: string, appType: AppId) => {
    queryClient.setQueryData<ProjectDto[]>(["projects"], (current) =>
      current?.map((project) =>
        project.projectPath === projectPath
          ? {
              ...project,
              routes: project.routes.filter((item) => item.appType !== appType),
            }
          : project,
      ),
    );
  };

  const updateProvider = async (
    projectPath: string,
    appType: AppId,
    providerId: string,
  ) => {
    try {
      if (!providerId) {
        await projectsApi.clearProvider(projectPath, appType);
        removeRouteFromCache(projectPath, appType);
      } else {
        const route = await projectsApi.setProvider({
          projectPath,
          appType,
          providerId,
        });
        setRouteInCache(projectPath, appType, route);
      }
      toast.success(
        t("projectManager.routeUpdated", {
          defaultValue: "项目供应商已更新",
        }),
      );
    } catch (error) {
      toast.error(
        extractErrorMessage(error) ||
          t("projectManager.routeUpdateFailed", {
            defaultValue: "项目供应商更新失败",
          }),
      );
    }
  };

  const updateForceModel = async (
    projectPath: string,
    appType: AppId,
    enabled: boolean,
    forceModel?: string | null,
  ) => {
    try {
      const route = await projectsApi.setForceModel({
        projectPath,
        appType,
        enabled,
        forceModel,
      });
      setRouteInCache(projectPath, appType, route);
      toast.success(
        t("projectManager.forceModelUpdated", {
          defaultValue: "强制路由模型已更新",
        }),
      );
    } catch (error) {
      toast.error(
        extractErrorMessage(error) ||
          t("projectManager.forceModelUpdateFailed", {
            defaultValue: "强制路由模型更新失败",
          }),
      );
    }
  };

  const addForceModel = async (model: string) => {
    try {
      const models = await projectsApi.addForceModel(model);
      queryClient.setQueryData(["project-force-models"], models);
    } catch (error) {
      toast.error(
        extractErrorMessage(error) ||
          t("projectManager.forceModelAddFailed", {
            defaultValue: "新增模型失败",
          }),
      );
      throw error;
    }
  };

  const deleteForceModel = async (model: string) => {
    try {
      const models = await projectsApi.deleteForceModel(model);
      queryClient.setQueryData(["project-force-models"], models);
    } catch (error) {
      toast.error(
        extractErrorMessage(error) ||
          t("projectManager.forceModelDeleteFailed", {
            defaultValue: "删除模型失败",
          }),
      );
      throw error;
    }
  };

  const providers = providerMaps[activeProjectApp] ?? {};

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-hidden px-6 pt-4 pb-8">
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-3">
        <div className="inline-flex gap-1 rounded-xl bg-muted p-1">
          {PROJECT_APPS.map((app) => {
            const isActive = activeProjectApp === app.id;
            return (
              <button
                key={app.id}
                type="button"
                onClick={() => setActiveProjectApp(app.id)}
                className={cn(
                  "inline-flex h-9 items-center gap-2 rounded-lg px-3 text-sm font-medium transition-all duration-200",
                  isActive
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:bg-background/50 hover:text-foreground",
                )}
              >
                <ProviderIcon icon={app.icon} name={app.label} size={16} />
                <span>{app.label}</span>
              </button>
            );
          })}
        </div>

        <div className="flex min-w-0 flex-1 items-center justify-end gap-2 sm:flex-none">
          <div className="relative min-w-0 flex-1 sm:w-64">
            <Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder={t("projectManager.searchPlaceholder", {
                defaultValue: "搜索项目",
              })}
              className="pl-9"
            />
          </div>
          <Button
            variant="outline"
            size="icon"
            onClick={() => void projects.refetch()}
            disabled={projects.isFetching}
            title={t("projectManager.refresh", { defaultValue: "刷新" })}
          >
            <RefreshCw
              className={cn("size-4", projects.isFetching && "animate-spin")}
            />
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto">
        {projects.isLoading && (
          <div className="rounded-xl border border-dashed p-10 text-center text-sm text-muted-foreground">
            {t("projectManager.loading", { defaultValue: "正在扫描项目..." })}
          </div>
        )}

        {projects.isError && (
          <div className="rounded-xl border border-dashed p-10 text-center text-sm text-destructive">
            {extractErrorMessage(projects.error)}
          </div>
        )}

        {!projects.isLoading &&
          !projects.isError &&
          filteredProjects.length === 0 && (
            <div className="rounded-xl border border-dashed p-10 text-center text-sm text-muted-foreground">
              {t("projectManager.empty", {
                defaultValue: "尚未发现带项目目录的会话",
              })}
            </div>
          )}

        {filteredProjects.map((project) => {
          const route = project.routes.find(
            (item) => item.appType === activeProjectApp && item.enabled,
          );
          const providerId = route?.providerId ?? "";
          const routeProviderMissing =
            Boolean(route?.providerId) && !providers[route!.providerId];
          const canUseRoute = Boolean(route) && !routeProviderMissing;
          const forceModel = route?.forceModel?.trim() ?? "";
          const latestModel = project.sessions.find(
            (session) =>
              session.providerId === activeProjectApp &&
              Boolean(session.lastModel),
          )?.lastModel;

          return (
            <div
              key={`${activeProjectApp}:${project.pathKey}`}
              className="group relative overflow-hidden rounded-xl border border-border bg-card text-card-foreground transition-all duration-300 hover:border-border-active hover:shadow-sm"
            >
              <div className="pointer-events-none absolute inset-0 bg-gradient-to-r from-primary/10 via-transparent to-transparent opacity-0 transition-opacity duration-300 group-hover:opacity-100" />
              <div className="relative flex items-center justify-between gap-4 px-4 py-3">
                <div className="flex min-w-0 flex-1 items-center gap-3">
                  <div className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-muted transition-transform duration-300 group-hover:scale-105">
                    <FolderOpen className="size-4" />
                  </div>
                  <div className="min-w-0">
                    <div className="truncate text-sm font-semibold">
                      {getBaseName(project.projectPath) || project.projectPath}
                    </div>
                    <div
                      className="truncate text-xs text-muted-foreground"
                      title={project.projectPath}
                    >
                      {project.projectPath}
                    </div>
                    {latestModel && (
                      <div className="mt-0.5 flex min-w-0 items-center gap-1 text-[9px] leading-none text-muted-foreground">
                        <span className="shrink-0">
                          {t("projectManager.lastModel", {
                            defaultValue: "最近模型",
                          })}
                        </span>
                        <span className="truncate rounded border border-border bg-muted px-1 py-px text-[8px] font-medium text-foreground/75">
                          {latestModel}
                        </span>
                      </div>
                    )}
                  </div>
                </div>

                <div className="flex shrink-0 items-center gap-2">
                  <select
                    value={providerId}
                    onChange={(event) =>
                      void updateProvider(
                        project.projectPath,
                        activeProjectApp,
                        event.target.value,
                      )
                    }
                    className={cn(
                      "h-9 w-[170px] shrink-0 rounded-lg border bg-background px-3 text-sm outline-none transition-colors focus:border-primary",
                      routeProviderMissing && "border-amber-500/60",
                    )}
                    title={t("projectManager.selectProvider", {
                      defaultValue: "选择项目供应商",
                    })}
                  >
                    <option value="">
                      {t("projectManager.followGlobal", {
                        defaultValue: "默认供应商",
                      })}
                    </option>
                    {routeProviderMissing && route && (
                      <option value={route.providerId}>
                        {route.providerId}（
                        {t("projectManager.providerMissing", {
                          defaultValue: "已不存在",
                        })}
                        ）
                      </option>
                    )}
                    {Object.entries(providers).map(([id, provider]) => (
                      <option key={id} value={id}>
                        {provider.name || id}
                      </option>
                    ))}
                  </select>

                  <div
                    className={cn(
                      "flex h-9 shrink-0 items-center gap-2",
                      !canUseRoute && "opacity-60",
                    )}
                    title={t("projectManager.forceModelHint", {
                      defaultValue:
                        "选择模型后开启强制路由；选择“关闭强制路由模型”恢复普通路由",
                    })}
                  >
                    <ForceModelDropdown
                      value={forceModel}
                      enabled={Boolean(route?.forceModelEnabled)}
                      models={forceModels.data ?? []}
                      selectionDisabled={!canUseRoute}
                      onDisable={() =>
                        void updateForceModel(
                          project.projectPath,
                          activeProjectApp,
                          false,
                          forceModel || null,
                        )
                      }
                      onSelect={(model) =>
                        void updateForceModel(
                          project.projectPath,
                          activeProjectApp,
                          true,
                          model,
                        )
                      }
                      onAdd={addForceModel}
                      onDelete={deleteForceModel}
                    />
                  </div>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
