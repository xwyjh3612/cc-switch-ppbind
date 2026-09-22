import { invoke } from "@tauri-apps/api/core";
import type { SessionMeta } from "@/types";

export interface ProjectProviderRoute {
  projectPathKey: string;
  projectPath: string;
  appType: string;
  providerId: string;
  enabled: boolean;
  forceModelEnabled: boolean;
  forceModel?: string | null;
  updatedAt: number;
}

export interface SessionProviderRoute {
  appType: string;
  sessionId: string;
  projectPathKey: string;
  providerId?: string | null;
  forceModel?: string | null;
  updatedAt: number;
}

export interface ProjectDto {
  pathKey: string;
  projectPath: string;
  sessions: SessionMeta[];
  routes: ProjectProviderRoute[];
  sessionRoutes: SessionProviderRoute[];
}

export const projectsApi = {
  async list(): Promise<ProjectDto[]> {
    return await invoke("list_projects");
  },
  async listRoutes(): Promise<ProjectProviderRoute[]> {
    return await invoke("list_project_routes");
  },
  async setProvider(options: {
    projectPath: string;
    appType: string;
    providerId: string;
  }): Promise<ProjectProviderRoute> {
    return await invoke("set_project_provider", {
      projectPath: options.projectPath,
      appType: options.appType,
      providerId: options.providerId,
    });
  },
  async clearProvider(projectPath: string, appType: string): Promise<boolean> {
    return await invoke("clear_project_provider", { projectPath, appType });
  },
  async resetProviders(appType: string): Promise<number> {
    return await invoke("reset_project_providers", { appType });
  },
  async listForceModels(appType: string): Promise<string[]> {
    return await invoke("list_project_force_models", { appType });
  },
  async addForceModel(appType: string, model: string): Promise<string[]> {
    return await invoke("add_project_force_model", { appType, model });
  },
  async deleteForceModel(appType: string, model: string): Promise<string[]> {
    return await invoke("delete_project_force_model", { appType, model });
  },
  async setSessionRoute(options: {
    projectPath: string;
    appType: string;
    sessionId: string;
    providerId?: string | null;
    forceModel?: string | null;
  }): Promise<SessionProviderRoute | null> {
    return await invoke("set_session_route", {
      projectPath: options.projectPath,
      appType: options.appType,
      sessionId: options.sessionId,
      providerId: options.providerId ?? null,
      forceModel: options.forceModel ?? null,
    });
  },
  async clearSessionRoute(
    appType: string,
    sessionId: string,
  ): Promise<boolean> {
    return await invoke("clear_session_route", { appType, sessionId });
  },
  async setForceModel(options: {
    projectPath: string;
    appType: string;
    enabled: boolean;
    forceModel?: string | null;
  }): Promise<ProjectProviderRoute> {
    return await invoke("set_project_force_model", {
      projectPath: options.projectPath,
      appType: options.appType,
      enabled: options.enabled,
      forceModel: options.forceModel ?? null,
    });
  },
};
