import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProjectManagerPage } from "@/components/projects/ProjectManagerPage";
import { projectsApi, providersApi, sessionsApi } from "@/lib/api";
import type { ProjectDto } from "@/lib/api/projects";

const { success, error } = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { success, error } }));
vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({
    isOpen,
    message,
    confirmText,
    pending,
    onConfirm,
    onCancel,
  }: {
    isOpen: boolean;
    message: string;
    confirmText: string;
    pending: boolean;
    onConfirm: () => void;
    onCancel: () => void;
  }) =>
    isOpen ? (
      <div role="dialog">
        <p>{message}</p>
        <button disabled={pending} onClick={onConfirm}>
          {confirmText}
        </button>
        <button disabled={pending} onClick={onCancel}>
          取消
        </button>
      </div>
    ) : null,
}));

describe("project session deletion", () => {
  let projects: ProjectDto[];
  beforeEach(() => {
    projects = [
      {
        pathKey: "project",
        projectPath: "C:/work/project",
        sessions: ["codex", "claude"].map((providerId) => ({
          providerId,
          sessionId: `${providerId}-1`,
          title: `${providerId} conversation`,
          sourcePath: `C:/sessions/${providerId}-1.jsonl`,
        })),
        routes: [],
        sessionRoutes: [],
      },
    ];
    vi.spyOn(projectsApi, "list").mockImplementation(async () => projects);
    vi.spyOn(projectsApi, "listForceModels").mockResolvedValue([]);
    vi.spyOn(providersApi, "getAll").mockResolvedValue({});
    vi.spyOn(projectsApi, "clearSessionRoute").mockResolvedValue(true);
    vi.spyOn(sessionsApi, "list").mockResolvedValue([]);
    vi.spyOn(sessionsApi, "delete").mockImplementation(async (input) => {
      projects = projects.map((project) => ({
        ...project,
        sessions: project.sessions.filter(
          (session) => session.sessionId !== input.sessionId,
        ),
      }));
      return true;
    });
  });
  afterEach(() => vi.restoreAllMocks());

  function renderPage() {
    const client = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    render(
      <QueryClientProvider client={client}>
        <ProjectManagerPage />
      </QueryClientProvider>,
    );
  }

  it.each(["codex", "claude"])(
    "deletes a %s session after confirmation",
    async (providerId) => {
      renderPage();
      await screen.findByText("project");
      if (providerId === "claude")
        fireEvent.click(screen.getByRole("button", { name: /Claude Code$/ }));
      fireEvent.click(screen.getByRole("button", { name: "展开会话" }));
      fireEvent.click(
        screen.getByRole("button", {
          name: `删除会话：${providerId} conversation`,
        }),
      );
      expect(sessionsApi.delete).not.toHaveBeenCalled();
      const dialog = screen.getByRole("dialog");
      expect(dialog).toHaveTextContent(`${providerId}-1`);
      fireEvent.click(within(dialog).getByRole("button", { name: "删除会话" }));
      await waitFor(() =>
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
      );
      expect(sessionsApi.delete).toHaveBeenCalledWith({
        providerId,
        sessionId: `${providerId}-1`,
        sourcePath: `C:/sessions/${providerId}-1.jsonl`,
      });
      expect(projectsApi.clearSessionRoute).toHaveBeenCalledWith(
        providerId,
        `${providerId}-1`,
      );
      expect(
        screen.queryByText(`${providerId} conversation`),
      ).not.toBeInTheDocument();
      expect(error).not.toHaveBeenCalled();
    },
  );

  it("keeps the session and restores the confirmation button when deletion fails", async () => {
    vi.mocked(sessionsApi.delete).mockRejectedValueOnce(
      new Error("file in use"),
    );
    renderPage();
    await screen.findByText("project");
    fireEvent.click(screen.getByRole("button", { name: "展开会话" }));
    fireEvent.click(
      screen.getByRole("button", { name: "删除会话：codex conversation" }),
    );
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "删除会话",
      }),
    );
    await waitFor(() => expect(error).toHaveBeenCalled());
    expect(screen.getByText("codex conversation")).toBeInTheDocument();
    expect(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "删除会话",
      }),
    ).not.toBeDisabled();
    expect(projectsApi.clearSessionRoute).not.toHaveBeenCalled();
  });
});
