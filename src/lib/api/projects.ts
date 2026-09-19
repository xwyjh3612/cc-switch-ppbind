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

export interface ProjectDto {
  pathKey: string;
  projectPath: string;
  sessions: SessionMeta[];
  routes: ProjectProviderRoute[];
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
  async listForceModels(): Promise<string[]> {
    return await invoke("list_project_force_models");
  },
  async addForceModel(model: string): Promise<string[]> {
    return await invoke("add_project_force_model", { model });
  },
  async deleteForceModel(model: string): Promise<string[]> {
    return await invoke("delete_project_force_model", { model });
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
