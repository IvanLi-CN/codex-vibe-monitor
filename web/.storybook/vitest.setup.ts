import * as a11yAddonAnnotations from "@storybook/addon-a11y/preview";
import { setProjectAnnotations } from "@storybook/react-vite";
import { beforeAll } from "vitest";
import preview from "./preview";

const project = setProjectAnnotations([a11yAddonAnnotations, preview]);

if (project.beforeAll) {
  beforeAll(project.beforeAll);
}
