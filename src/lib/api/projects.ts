import { invoke } from "@tauri-apps/api/core";
import type { SessionMeta } from "@/types";

export interface ProjectProviderRoute {
  projectPathKey: string;
  projectPath: string;
  appType: string;
  providerId: string;
  enabled: boolean;
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
};
