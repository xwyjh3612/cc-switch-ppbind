import { useMemo, useRef, useState } from "react";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  FolderOpen,
  MessageSquare,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
  Trash2,
} from "lucide-react";
import { projectsApi, providersApi } from "@/lib/api";
import type {
  ProjectDto,
  ProjectProviderRoute,
  SessionProviderRoute,
} from "@/lib/api/projects";
import type { AppId } from "@/lib/api/types";
import type { Provider } from "@/types";
import {
  formatRelativeTime,
  formatSessionTitle,
  getBaseName,
  getSessionKey,
} from "@/components/sessions/utils";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverAnchor,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { extractErrorMessage } from "@/utils/errorUtils";

// 顶部预留 64px 固定页头，并给视口边缘留 8px 间距。
const PROJECT_POPOVER_COLLISION_PADDING = {
  top: 72,
  right: 8,
  bottom: 8,
  left: 8,
};

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
  defaultLabel?: string;
  placeholder?: string;
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
  defaultLabel,
  placeholder,
}: ForceModelDropdownProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [pending, setPending] = useState(false);
  const inputAnchorRef = useRef<HTMLDivElement>(null);
  const allModels =
    value && !models.includes(value) ? [value, ...models] : models;
  const normalizedSearch = search.trim().toLowerCase();
  const visibleModels = normalizedSearch
    ? allModels.filter((model) =>
        model.toLowerCase().includes(normalizedSearch),
      )
    : allModels;
  const orderedModels =
    normalizedSearch || !value
      ? visibleModels
      : visibleModels.includes(value)
        ? [value, ...visibleModels.filter((model) => model !== value)]
        : visibleModels;
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
        : placeholder ||
          t("projectManager.forceModelPlaceholder", {
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
        <div ref={inputAnchorRef} className="relative w-[170px] shrink-0">
          <Input
            value={search}
            onFocus={() => {
              setSearch("");
              setOpen(true);
            }}
            onClick={() => {
              if (!open) {
                setSearch("");
                setOpen(true);
              }
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
        collisionPadding={PROJECT_POPOVER_COLLISION_PADDING}
        align="end"
        className="w-[240px] overflow-hidden p-0"
        onInteractOutside={(event) => {
          if (inputAnchorRef.current?.contains(event.target as Node)) {
            event.preventDefault();
          }
        }}
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <div
          className="overflow-y-auto overscroll-contain"
          style={{
            maxHeight:
              "min(60vh, 420px, var(--radix-popper-available-height, 420px))",
          }}
        >
          <button
            type="button"
            disabled={selectionDisabled}
            onClick={() => {
              onDisable();
              closePicker();
            }}
            className={cn(
              "flex w-full items-center gap-2 rounded-none px-3 py-1.5 text-left text-xs hover:bg-muted disabled:cursor-not-allowed disabled:opacity-60",
              !enabled && "bg-muted/60",
            )}
          >
            <Check
              className={cn(
                "size-3.5 shrink-0 text-emerald-500",
                enabled && "opacity-0",
              )}
            />
            <span className="min-w-0 flex-1 truncate">
              {defaultLabel ||
                t("projectManager.forceModelDisable", {
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
          {orderedModels.map((model) => {
            const selected = enabled && model === value;
            const stale = !models.includes(model);
            const lastUsed = !enabled && model === value;
            return (
              <div
                key={model}
                className={cn(
                  "group flex items-center gap-1 rounded-none hover:bg-muted",
                  selected && "bg-muted/60",
                )}
              >
                <button
                  type="button"
                  disabled={selectionDisabled}
                  onClick={() => handleSelect(model)}
                  className="flex min-w-0 flex-1 items-center gap-2 px-3 py-1.5 text-left text-xs disabled:cursor-not-allowed disabled:opacity-60"
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
                    className="mr-2 rounded p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100 disabled:opacity-40"
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
interface ProviderDropdownOption {
  id: string;
  name: string;
  missing?: boolean;
}

interface ProviderDropdownProps {
  value: string;
  missingProviderId?: string;
  options: ProviderDropdownOption[];
  onChange: (providerId: string) => void;
  defaultLabel?: string;
}

function ProviderDropdown({
  value,
  missingProviderId,
  options,
  onChange,
  defaultLabel,
}: ProviderDropdownProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const inputAnchorRef = useRef<HTMLDivElement>(null);
  const allOptions =
    missingProviderId &&
    !options.some((option) => option.id === missingProviderId)
      ? [
          { id: missingProviderId, name: missingProviderId, missing: true },
          ...options,
        ]
      : options;
  const normalizedSearch = search.trim().toLowerCase();
  const visibleOptions = normalizedSearch
    ? allOptions.filter(
        (option) =>
          option.id.toLowerCase().includes(normalizedSearch) ||
          option.name.toLowerCase().includes(normalizedSearch),
      )
    : allOptions;
  const selectedOption = allOptions.find((option) => option.id === value);
  const orderedOptions =
    normalizedSearch || !value || !selectedOption
      ? visibleOptions
      : visibleOptions.some((option) => option.id === value)
        ? [
            selectedOption,
            ...visibleOptions.filter((option) => option.id !== value),
          ]
        : visibleOptions;
  const triggerText = value
    ? selectedOption?.missing
      ? `${selectedOption.name}（${t("projectManager.providerMissing", {
          defaultValue: "已不存在",
        })}）`
      : selectedOption?.name || value
    : defaultLabel ||
      t("projectManager.followGlobal", {
        defaultValue: "默认供应商",
      });

  const closePicker = () => {
    setOpen(false);
    setSearch("");
  };

  const handleSelect = (providerId: string) => {
    onChange(providerId);
    closePicker();
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
        <div ref={inputAnchorRef} className="relative w-[170px] shrink-0">
          <Input
            value={search}
            onFocus={() => {
              setSearch("");
              setOpen(true);
            }}
            onClick={() => {
              if (!open) {
                setSearch("");
                setOpen(true);
              }
            }}
            onChange={(event) => {
              setSearch(event.target.value);
              setOpen(true);
            }}
            placeholder={triggerText}
            autoComplete="off"
            className="h-9 w-full pr-8 text-sm"
            title={t("projectManager.selectProvider", {
              defaultValue: "选择项目供应商",
            })}
          />
          <ChevronDown className="pointer-events-none absolute right-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        </div>
      </PopoverAnchor>
      <PopoverContent
        collisionPadding={PROJECT_POPOVER_COLLISION_PADDING}
        align="end"
        className="w-[240px] overflow-hidden p-0"
        onInteractOutside={(event) => {
          if (inputAnchorRef.current?.contains(event.target as Node)) {
            event.preventDefault();
          }
        }}
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <div
          className="overflow-y-auto overscroll-contain"
          style={{
            maxHeight:
              "min(60vh, 420px, var(--radix-popper-available-height, 420px))",
          }}
        >
          <button
            type="button"
            onClick={() => handleSelect("")}
            className={cn(
              "flex w-full items-center gap-2 rounded-none px-3 py-1.5 text-left text-xs hover:bg-muted",
              !value && "bg-muted/60",
            )}
          >
            <Check
              className={cn(
                "size-3.5 shrink-0 text-emerald-500",
                value && "opacity-0",
              )}
            />
            <span className="min-w-0 flex-1 truncate">
              {defaultLabel ||
                t("projectManager.followGlobal", {
                  defaultValue: "默认供应商",
                })}
            </span>
          </button>

          {visibleOptions.length > 0 && (
            <div className="my-1 border-t border-border/60" />
          )}

          {normalizedSearch && visibleOptions.length === 0 && (
            <div className="px-2 py-5 text-center text-xs text-muted-foreground">
              {t("projectManager.providerEmpty", {
                defaultValue: "暂无匹配供应商",
              })}
            </div>
          )}

          {orderedOptions.map((option) => {
            const selected = option.id === value;
            return (
              <button
                key={option.id}
                type="button"
                onClick={() => handleSelect(option.id)}
                className={cn(
                  "flex w-full items-center gap-2 rounded-none px-3 py-1.5 text-left text-xs hover:bg-muted",
                  selected && "bg-muted/60",
                )}
              >
                <Check
                  className={cn(
                    "size-3.5 shrink-0 text-emerald-500",
                    !selected && "opacity-0",
                  )}
                />
                <span className="min-w-0 flex-1 truncate">{option.name}</span>
                {option.missing && (
                  <span className="shrink-0 text-[9px] text-amber-500">
                    {t("projectManager.providerMissing", {
                      defaultValue: "已不存在",
                    })}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
const SESSION_PREVIEW_LIMIT = 5;

type PendingProjectRouteChange =
  | {
      kind: "provider";
      project: ProjectDto;
      appType: AppId;
      providerId: string;
      sessionCount: number;
    }
  | {
      kind: "force-model";
      project: ProjectDto;
      appType: AppId;
      enabled: boolean;
      forceModel?: string | null;
      sessionCount: number;
    };

const projectAppSessionKey = (appType: AppId, pathKey: string) =>
  `${appType}:${pathKey}`;

export function ProjectManagerPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [activeProjectApp, setActiveProjectApp] = useState<AppId>("codex");
  const [resetTargetApp, setResetTargetApp] = useState<AppId | null>(null);
  const [resettingProviders, setResettingProviders] = useState(false);
  const [expandedProjectKeys, setExpandedProjectKeys] = useState<Set<string>>(
    () => new Set(),
  );
  const [showAllSessionKeys, setShowAllSessionKeys] = useState<Set<string>>(
    () => new Set(),
  );
  const [pendingRouteChange, setPendingRouteChange] =
    useState<PendingProjectRouteChange | null>(null);
  const [applyingRouteChange, setApplyingRouteChange] = useState(false);

  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => projectsApi.list(),
  });

  const forceModels = useQuery({
    queryKey: ["project-force-models", activeProjectApp],
    queryFn: () => projectsApi.listForceModels(activeProjectApp),
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

  const activeRouteCount = useMemo(
    () =>
      (projects.data ?? []).filter((project) =>
        project.routes.some(
          (route) => route.appType === activeProjectApp && route.enabled,
        ),
      ).length,
    [activeProjectApp, projects.data],
  );

  const resetRouteCount = useMemo(
    () =>
      resetTargetApp
        ? (projects.data ?? []).filter((project) =>
            project.routes.some(
              (route) => route.appType === resetTargetApp && route.enabled,
            ),
          ).length
        : 0,
    [projects.data, resetTargetApp],
  );

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
              sessionRoutes: project.sessionRoutes.filter(
                (item) => item.appType !== appType,
              ),
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
              sessionRoutes: project.sessionRoutes.filter(
                (item) => item.appType !== appType,
              ),
            }
          : project,
      ),
    );
  };

  const setSessionRouteInCache = (
    projectPathKey: string,
    appType: AppId,
    sessionId: string,
    route: SessionProviderRoute | null,
  ) => {
    queryClient.setQueryData<ProjectDto[]>(["projects"], (current) =>
      current?.map((project) => {
        if (project.pathKey !== projectPathKey) return project;
        const remaining = project.sessionRoutes.filter(
          (item) => item.appType !== appType || item.sessionId !== sessionId,
        );
        return {
          ...project,
          sessionRoutes: route ? [...remaining, route] : remaining,
        };
      }),
    );
  };

  const updateSessionRoute = async (
    project: ProjectDto,
    appType: AppId,
    sessionId: string,
    providerId: string | null,
    forceModel: string | null,
  ) => {
    try {
      const route = await projectsApi.setSessionRoute({
        projectPath: project.projectPath,
        appType,
        sessionId,
        providerId,
        forceModel,
      });
      setSessionRouteInCache(project.pathKey, appType, sessionId, route);
      toast.success(
        t("projectManager.sessionRouteUpdated", {
          defaultValue: "会话路由已更新",
        }),
      );
    } catch (error) {
      toast.error(
        extractErrorMessage(error) ||
          t("projectManager.sessionRouteUpdateFailed", {
            defaultValue: "会话路由更新失败",
          }),
      );
    }
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

  const addForceModel = async (appType: AppId, model: string) => {
    try {
      const models = await projectsApi.addForceModel(appType, model);
      queryClient.setQueryData(["project-force-models", appType], models);
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

  const deleteForceModel = async (appType: AppId, model: string) => {
    try {
      const models = await projectsApi.deleteForceModel(appType, model);
      queryClient.setQueryData(["project-force-models", appType], models);
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

  const resetAllProviders = async () => {
    if (!resetTargetApp || resettingProviders) return;

    const appType = resetTargetApp;
    setResettingProviders(true);
    try {
      const count = await projectsApi.resetProviders(appType);
      queryClient.setQueryData<ProjectDto[]>(["projects"], (current) =>
        current?.map((project) => ({
          ...project,
          routes: project.routes.filter((route) => route.appType !== appType),
          sessionRoutes: project.sessionRoutes.filter(
            (route) => route.appType !== appType,
          ),
        })),
      );
      setResetTargetApp(null);
      toast.success(
        t("projectManager.resetProvidersSuccess", {
          defaultValue: "已将 {{count}} 个项目恢复为默认供应商",
          count,
        }),
      );
    } catch (error) {
      toast.error(
        extractErrorMessage(error) ||
          t("projectManager.resetProvidersFailed", {
            defaultValue: "批量恢复默认供应商失败",
          }),
      );
    } finally {
      setResettingProviders(false);
    }
  };

  const requestProviderChange = (
    project: ProjectDto,
    appType: AppId,
    providerId: string,
  ) => {
    const sessionCount = project.sessionRoutes.filter(
      (route) => route.appType === appType,
    ).length;
    if (sessionCount > 0) {
      setPendingRouteChange({
        kind: "provider",
        project,
        appType,
        providerId,
        sessionCount,
      });
      return;
    }
    void updateProvider(project.projectPath, appType, providerId);
  };

  const requestForceModelChange = (
    project: ProjectDto,
    appType: AppId,
    enabled: boolean,
    forceModel?: string | null,
  ) => {
    const sessionCount = project.sessionRoutes.filter(
      (route) => route.appType === appType,
    ).length;
    if (sessionCount > 0) {
      setPendingRouteChange({
        kind: "force-model",
        project,
        appType,
        enabled,
        forceModel,
        sessionCount,
      });
      return;
    }
    void updateForceModel(project.projectPath, appType, enabled, forceModel);
  };

  const applyPendingRouteChange = async () => {
    if (!pendingRouteChange || applyingRouteChange) return;
    setApplyingRouteChange(true);
    try {
      if (pendingRouteChange.kind === "provider") {
        await updateProvider(
          pendingRouteChange.project.projectPath,
          pendingRouteChange.appType,
          pendingRouteChange.providerId,
        );
      } else {
        await updateForceModel(
          pendingRouteChange.project.projectPath,
          pendingRouteChange.appType,
          pendingRouteChange.enabled,
          pendingRouteChange.forceModel,
        );
      }
      setPendingRouteChange(null);
    } finally {
      setApplyingRouteChange(false);
    }
  };

  const toggleProjectExpanded = (key: string) => {
    setExpandedProjectKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const toggleShowAllSessions = (key: string) => {
    setShowAllSessionKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const providers = providerMaps[activeProjectApp] ?? {};

  const providerOptions = useMemo(
    () =>
      Object.entries(providers).map(([id, provider]) => ({
        id,
        name: provider.name || id,
      })),
    [providers],
  );

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
          <Button
            variant="outline"
            onClick={() => setResetTargetApp(activeProjectApp)}
            disabled={activeRouteCount === 0 || resettingProviders}
            title={t("projectManager.resetProvidersHint", {
              defaultValue: "将当前项目类型的所有项目恢复为默认供应商",
            })}
          >
            <RotateCcw className="mr-2 size-4" />
            {t("projectManager.resetProviders", {
              defaultValue: "全部恢复默认",
            })}
          </Button>
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
          const projectKey = projectAppSessionKey(
            activeProjectApp,
            project.pathKey,
          );
          const expanded = expandedProjectKeys.has(projectKey);
          const showAllSessions = showAllSessionKeys.has(projectKey);
          const appSessions = project.sessions.filter(
            (session) => session.providerId === activeProjectApp,
          );
          const visibleSessions = showAllSessions
            ? appSessions
            : appSessions.slice(0, SESSION_PREVIEW_LIMIT);
          const sessionOverrideCount = project.sessionRoutes.filter(
            (sessionRoute) => sessionRoute.appType === activeProjectApp,
          ).length;
          const latestModel = appSessions.find((session) =>
            Boolean(session.lastModel),
          )?.lastModel;

          return (
            <div
              key={projectKey}
              className="group relative overflow-hidden rounded-xl border border-border bg-card text-card-foreground transition-all duration-300 hover:border-border-active hover:shadow-sm"
            >
              <div className="pointer-events-none absolute inset-0 bg-gradient-to-r from-primary/10 via-transparent to-transparent opacity-0 transition-opacity duration-300 group-hover:opacity-100" />
              <div className="relative flex items-center justify-between gap-4 px-4 py-3">
                <div className="flex min-w-0 flex-1 items-center gap-3">
                  <button
                    type="button"
                    onClick={() => toggleProjectExpanded(projectKey)}
                    className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-muted text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground"
                    title={t("projectManager.toggleSessions", {
                      defaultValue: expanded ? "收起会话" : "展开会话",
                    })}
                  >
                    {expanded ? (
                      <ChevronDown className="size-4" />
                    ) : (
                      <ChevronRight className="size-4" />
                    )}
                  </button>
                  <div className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-muted transition-transform duration-300 group-hover:scale-105">
                    <FolderOpen className="size-4" />
                  </div>
                  <div className="min-w-0">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate text-sm font-semibold">
                        {getBaseName(project.projectPath) ||
                          project.projectPath}
                      </span>
                      <span className="shrink-0 text-[10px] font-normal text-muted-foreground">
                        {t("projectManager.sessionCount", {
                          defaultValue: "{{count}} 个会话",
                          count: appSessions.length,
                        })}
                      </span>
                      {sessionOverrideCount > 0 && (
                        <span className="shrink-0 rounded bg-primary/10 px-1 py-px text-[9px] text-primary">
                          {t("projectManager.sessionOverrideCount", {
                            defaultValue: "{{count}} 个已单独设置",
                            count: sessionOverrideCount,
                          })}
                        </span>
                      )}
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
                  <ProviderDropdown
                    value={providerId}
                    missingProviderId={
                      routeProviderMissing ? route?.providerId : undefined
                    }
                    options={providerOptions}
                    onChange={(nextProviderId) =>
                      requestProviderChange(
                        project,
                        activeProjectApp,
                        nextProviderId,
                      )
                    }
                  />

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
                        requestForceModelChange(
                          project,
                          activeProjectApp,
                          false,
                          forceModel || null,
                        )
                      }
                      onSelect={(model) =>
                        requestForceModelChange(
                          project,
                          activeProjectApp,
                          true,
                          model,
                        )
                      }
                      onAdd={(model) => addForceModel(activeProjectApp, model)}
                      onDelete={(model) =>
                        deleteForceModel(activeProjectApp, model)
                      }
                    />
                  </div>
                </div>
              </div>

              {expanded && (
                <div className="relative border-t border-border/70 bg-muted/20 px-4 py-3">
                  {appSessions.length === 0 ? (
                    <div className="py-4 text-center text-xs text-muted-foreground">
                      {t("projectManager.noSessions", {
                        defaultValue: "该项目下暂无会话",
                      })}
                    </div>
                  ) : (
                    <div className="space-y-1.5">
                      {visibleSessions.map((session) => {
                        const sessionRoute = project.sessionRoutes.find(
                          (item) =>
                            item.appType === activeProjectApp &&
                            item.sessionId === session.sessionId,
                        );
                        const sessionProviderId =
                          sessionRoute?.providerId?.trim() ?? "";
                        const sessionProviderMissing =
                          Boolean(sessionProviderId) &&
                          !providers[sessionProviderId];
                        const sessionModel =
                          sessionRoute?.forceModel?.trim() ?? "";
                        const lastActive =
                          session.lastActiveAt || session.createdAt;

                        return (
                          <div
                            key={getSessionKey(session)}
                            className="flex flex-col gap-2 rounded-lg border border-transparent px-2 py-2 transition-colors hover:border-border/70 hover:bg-background/70 sm:flex-row sm:items-center sm:justify-between"
                          >
                            <div className="flex min-w-0 flex-1 items-start gap-2">
                              <MessageSquare className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
                              <div className="min-w-0">
                                <div
                                  className="truncate text-xs font-medium"
                                  title={formatSessionTitle(session)}
                                >
                                  {formatSessionTitle(session)}
                                </div>
                                <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
                                  <span>
                                    {lastActive
                                      ? formatRelativeTime(lastActive, t)
                                      : t("common.unknown")}
                                  </span>
                                  {session.lastModel && (
                                    <>
                                      <span>·</span>
                                      <span className="truncate">
                                        {session.lastModel}
                                      </span>
                                    </>
                                  )}
                                </div>
                              </div>
                            </div>

                            <div className="flex shrink-0 flex-wrap items-center gap-2 pl-5 sm:pl-0">
                              <ProviderDropdown
                                value={sessionProviderId}
                                missingProviderId={
                                  sessionProviderMissing
                                    ? sessionProviderId
                                    : undefined
                                }
                                options={providerOptions}
                                defaultLabel={t(
                                  "projectManager.followProject",
                                  { defaultValue: "跟随项目" },
                                )}
                                onChange={(nextProviderId) =>
                                  void updateSessionRoute(
                                    project,
                                    activeProjectApp,
                                    session.sessionId,
                                    nextProviderId || null,
                                    sessionModel || null,
                                  )
                                }
                              />

                              <ForceModelDropdown
                                value={sessionModel}
                                enabled={Boolean(sessionModel)}
                                models={forceModels.data ?? []}
                                selectionDisabled={false}
                                defaultLabel={t(
                                  "projectManager.followProject",
                                  { defaultValue: "跟随项目" },
                                )}
                                placeholder={t("projectManager.followProject", {
                                  defaultValue: "跟随项目",
                                })}
                                onDisable={() =>
                                  void updateSessionRoute(
                                    project,
                                    activeProjectApp,
                                    session.sessionId,
                                    sessionProviderId || null,
                                    null,
                                  )
                                }
                                onSelect={(model) =>
                                  void updateSessionRoute(
                                    project,
                                    activeProjectApp,
                                    session.sessionId,
                                    sessionProviderId || null,
                                    model,
                                  )
                                }
                                onAdd={(model) =>
                                  addForceModel(activeProjectApp, model)
                                }
                                onDelete={(model) =>
                                  deleteForceModel(activeProjectApp, model)
                                }
                              />
                            </div>
                          </div>
                        );
                      })}

                      {appSessions.length > SESSION_PREVIEW_LIMIT && (
                        <button
                          type="button"
                          onClick={() => toggleShowAllSessions(projectKey)}
                          className="mx-auto flex items-center gap-1 rounded px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-background/70 hover:text-foreground"
                        >
                          {showAllSessions ? (
                            <ChevronUp className="size-3.5" />
                          ) : (
                            <ChevronDown className="size-3.5" />
                          )}
                          {showAllSessions
                            ? t("projectManager.showRecentSessions", {
                                defaultValue: "只显示最近 {{count}} 条",
                                count: SESSION_PREVIEW_LIMIT,
                              })
                            : t("projectManager.showAllSessions", {
                                defaultValue: "显示全部 {{count}} 条",
                                count: appSessions.length,
                              })}
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      <ConfirmDialog
        isOpen={pendingRouteChange !== null}
        title={t("projectManager.resetSessionOverridesTitle", {
          defaultValue: "同时重置会话指定",
        })}
        message={t("projectManager.resetSessionOverridesConfirm", {
          defaultValue:
            "修改项目“{{project}}”的{{field}}后，{{count}} 个会话的单独渠道或模型设置将恢复为跟随项目。",
          project: pendingRouteChange
            ? getBaseName(pendingRouteChange.project.projectPath) ||
              pendingRouteChange.project.projectPath
            : "",
          field:
            pendingRouteChange?.kind === "provider"
              ? t("projectManager.providerField", {
                  defaultValue: "供应商",
                })
              : t("projectManager.forceModelField", {
                  defaultValue: "强制路由模型",
                }),
          count: pendingRouteChange?.sessionCount ?? 0,
        })}
        confirmText={t("projectManager.confirmAndReset", {
          defaultValue: "确认并重置",
        })}
        pending={applyingRouteChange}
        onConfirm={() => void applyPendingRouteChange()}
        onCancel={() => {
          if (!applyingRouteChange) setPendingRouteChange(null);
        }}
      />

      <ConfirmDialog
        isOpen={resetTargetApp !== null}
        title={t("projectManager.resetProvidersTitle", {
          defaultValue: "恢复默认供应商",
        })}
        message={t("projectManager.resetProvidersConfirm", {
          defaultValue:
            "将把 {{app}} 下的 {{count}} 个项目全部恢复为默认供应商，并关闭项目与会话级的强制模型路由。\n\n此操作不会影响另一项目类型。",
          app: resetTargetApp === "claude" ? "Claude Code" : "Codex",
          count: resetRouteCount,
        })}
        confirmText={t("projectManager.resetProvidersConfirmButton", {
          defaultValue: "确认重置",
        })}
        pending={resettingProviders}
        onConfirm={() => void resetAllProviders()}
        onCancel={() => {
          if (!resettingProviders) setResetTargetApp(null);
        }}
      />
    </div>
  );
}
