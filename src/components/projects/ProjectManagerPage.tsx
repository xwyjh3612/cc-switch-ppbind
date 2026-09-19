import { useMemo, useState } from "react";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { FolderOpen, RefreshCw, Search } from "lucide-react";
import { projectsApi, providersApi } from "@/lib/api";
import type { AppId } from "@/lib/api/types";
import type { Provider } from "@/types";
import { getBaseName } from "@/components/sessions/utils";
import { ProviderIcon } from "@/components/ProviderIcon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { extractErrorMessage } from "@/utils/errorUtils";

const PROJECT_APPS: Array<{ id: AppId; label: string; icon: string }> = [
  { id: "codex", label: "Codex", icon: "openai" },
  { id: "claude", label: "Claude Code", icon: "claude" },
];

export function ProjectManagerPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [activeProjectApp, setActiveProjectApp] = useState<AppId>("codex");

  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => projectsApi.list(),
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

  const updateProvider = async (
    projectPath: string,
    appType: AppId,
    providerId: string,
  ) => {
    try {
      if (!providerId) {
        await projectsApi.clearProvider(projectPath, appType);
      } else {
        await projectsApi.setProvider({ projectPath, appType, providerId });
      }
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
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
            title={t("common.refresh", { defaultValue: "刷新" })}
          >
            <RefreshCw
              className={projects.isFetching ? "size-4 animate-spin" : "size-4"}
            />
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
        {projects.isLoading && (
          <div className="py-10 text-center text-sm text-muted-foreground">
            {t("common.loading", { defaultValue: "加载中…" })}
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
          const latestModel = project.sessions.find(
            (session) =>
              session.providerId === activeProjectApp && Boolean(session.lastModel),
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
                      <div className="mt-1 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                        <span className="shrink-0">
                          {t("projectManager.lastModel", {
                            defaultValue: "最近模型",
                          })}
                        </span>
                        <span className="truncate rounded-md border border-border bg-muted px-1.5 py-0.5 font-medium text-foreground/80">
                          {latestModel}
                        </span>
                      </div>
                    )}
                  </div>
                </div>

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
                    "h-9 w-[190px] shrink-0 rounded-lg border bg-background px-3 text-sm outline-none transition-colors focus:border-primary",
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
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
