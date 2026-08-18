declare module "frappe-gantt" {
  export interface GanttTask {
    id: string;
    name: string;
    start: Date | string;
    end: Date | string;
    progress?: number;
    dependencies?: string | string[];
    custom_class?: string;
  }

  export interface GanttViewMode {
    name: string;
    padding: string | [string, string];
    step: string;
    date_format?: string;
    column_width?: number;
    snap_at?: string;
    lower_text?: string | ((date: Date, previousDate?: Date, language?: string) => string);
    upper_text?: string | ((date: Date, previousDate?: Date, language?: string) => string);
    upper_text_frequency?: number;
    thick_line?: (date: Date) => boolean;
  }

  export interface GanttOptions {
    view_mode?: string;
    view_modes?: GanttViewMode[];
    column_width?: number;
    bar_height?: number;
    bar_corner_radius?: number;
    padding?: number;
    upper_header_height?: number;
    lower_header_height?: number;
    container_height?: number | "auto";
    infinite_padding?: boolean;
    lines?: "none" | "vertical" | "horizontal" | "both";
    holidays?: Record<string, unknown>;
    readonly?: boolean;
    today_button?: boolean;
    scroll_to?: "today" | "start" | "end" | string | null;
    popup?: false | ((context: unknown) => string | false | undefined);
    on_click?: (task: GanttTask) => void;
  }

  export default class Gantt {
    constructor(
      target: string | HTMLElement | SVGElement,
      tasks: GanttTask[],
      options?: GanttOptions,
    );
    refresh(tasks: GanttTask[]): void;
    change_view_mode(mode: string | GanttViewMode, maintainPosition?: boolean): void;
  }
}
