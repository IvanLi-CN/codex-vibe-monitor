import type * as React from "react";
import { useLayoutEffect, useState } from "react";
import {
  ACCOUNT_CARD_HERO_SINGLE_COLUMN_BREAKPOINT_PX,
  ACCOUNT_CARD_HERO_TWO_COLUMN_BREAKPOINT_PX,
  ACCOUNT_CARD_RECENT_DETAILS_SPLIT_BREAKPOINT_PX,
  ACCOUNT_CARD_RECENT_STACK_BREAKPOINT_PX,
  ACCOUNT_CARD_STACKED_HEADER_BREAKPOINT_PX,
} from "./DashboardWorkingConversationsSection";

export function useDashboardWorkingAccountCardWidth(cardRef: React.RefObject<HTMLElement | null>) {
  const [cardWidth, setCardWidth] = useState(0);
  useLayoutEffect(() => {
    const card = cardRef.current;
    if (!card) return undefined;
    const updateCardWidth = () => {
      const nextWidth = card.clientWidth;
      setCardWidth((current) => (Math.abs(current - nextWidth) > 0.5 ? nextWidth : current));
    };
    updateCardWidth();
    const frame = window.requestAnimationFrame(updateCardWidth);
    window.addEventListener("resize", updateCardWidth);
    if (typeof ResizeObserver === "undefined") {
      return () => {
        window.cancelAnimationFrame(frame);
        window.removeEventListener("resize", updateCardWidth);
      };
    }
    const observer = new ResizeObserver(updateCardWidth);
    observer.observe(card);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", updateCardWidth);
      observer.disconnect();
    };
  }, [cardRef]);
  return cardWidth;
}

export function buildDashboardWorkingAccountCardLayout(cardWidth: number) {
  const headerLayout =
    cardWidth > 0 && cardWidth < ACCOUNT_CARD_STACKED_HEADER_BREAKPOINT_PX ? "stacked" : "split";
  return {
    headerLayout,
    inlineMetricLayout: headerLayout === "stacked" ? "three-columns" : "inline",
    heroMetricColumnCount:
      cardWidth > 0 && cardWidth < ACCOUNT_CARD_HERO_SINGLE_COLUMN_BREAKPOINT_PX
        ? 1
        : cardWidth > 0 && cardWidth < ACCOUNT_CARD_HERO_TWO_COLUMN_BREAKPOINT_PX
          ? 2
          : 4,
    recentBreakdownLayout:
      cardWidth > 0 && cardWidth < ACCOUNT_CARD_RECENT_STACK_BREAKPOINT_PX ? "stacked" : "inline",
    recentDetailsLayout:
      cardWidth > 0 && cardWidth >= ACCOUNT_CARD_RECENT_DETAILS_SPLIT_BREAKPOINT_PX
        ? "split"
        : "stacked",
  } as const;
}
