# Compiled helpers: bindable list row + animated wallpaper effects.
if (-not ('OptiRow' -as [type])) {
    Add-Type -TypeDefinition @'
using System.ComponentModel;
public class OptiRow : INotifyPropertyChanged {
    public event PropertyChangedEventHandler PropertyChanged;
    void N(string p) { var h = PropertyChanged; if (h != null) h(this, new PropertyChangedEventArgs(p)); }
    string _name, _desc, _group, _risk, _extra; bool _chk;
    public string Id { get; set; }
    public object Tag { get; set; }
    public string Name  { get { return _name;  } set { _name  = value; N("Name");  } }
    public string Desc  { get { return _desc;  } set { _desc  = value; N("Desc");  } }
    public string Group { get { return _group; } set { _group = value; N("Group"); } }
    public string Risk  { get { return _risk;  } set { _risk  = value; N("Risk"); N("RiskText"); } }
    public string Extra { get { return _extra; } set { _extra = value; N("Extra"); N("ExtraText"); } }
    public string RiskText  { get { return (_risk  ?? "").ToLower(); } }
    public string ExtraText { get { return (_extra ?? "").ToLower(); } }
    public bool IsChecked { get { return _chk; } set { _chk = value; N("IsChecked"); } }
}
'@
}

if (-not ('OptiFx' -as [type])) {
    # One element draws every particle in a single OnRender pass; far cheaper than hundreds of animated shapes.
    Add-Type -ReferencedAssemblies PresentationCore, PresentationFramework, WindowsBase, System.Xaml -TypeDefinition @'
using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

public class OptiFx : FrameworkElement {
    class P { public double X, Y, VX, VY, S, Ph; public int K; }
    const int Levels = 16;

    readonly Random rnd = new Random();
    P[] ps = new P[0];
    string mode = "none";
    Color c1 = Colors.White, c2 = Colors.White;
    Brush[] ramp1 = new Brush[Levels], ramp2 = new Brush[Levels], aurora = new Brush[4];
    DateTime last = DateTime.MinValue;
    bool hooked;
    double t, w = 1280, h = 820;

    public OptiFx(Canvas canvas) {
        IsHitTestVisible = false;
        canvas.Children.Add(this);
        canvas.SizeChanged += delegate(object s, SizeChangedEventArgs e) { w = e.NewSize.Width; h = e.NewSize.Height; Width = w; Height = h; Rebuild(); };
    }

    public void SetColors(Color a, Color b) { c1 = a; c2 = b; Rebuild(); }

    public void SetMode(string m) {
        mode = m; Rebuild();
        if (!hooked) { CompositionTarget.Rendering += Tick; hooked = true; }
    }

    static Brush Solid(Color c, byte alpha) {
        var b = new SolidColorBrush(Color.FromArgb(alpha, c.R, c.G, c.B)); b.Freeze(); return b;
    }

    double R(double a, double b) { return a + rnd.NextDouble() * (b - a); }

    void Rebuild() {
        for (int i = 0; i < Levels; i++) {
            byte al = (byte)(255 * (i + 1) / Levels);
            ramp1[i] = Solid(c1, al); ramp2[i] = Solid(c2, al);
        }
        for (int i = 0; i < 4; i++) {
            Color c = i % 2 == 0 ? c1 : c2;
            var g = new RadialGradientBrush();
            g.GradientStops.Add(new GradientStop(Color.FromArgb(95, c.R, c.G, c.B), 0));
            g.GradientStops.Add(new GradientStop(Color.FromArgb(0, c.R, c.G, c.B), 1));
            g.Freeze(); aurora[i] = g;
        }
        int n = mode == "stars" ? 130 : mode == "rain" ? 90 : mode == "embers" ? 60 : mode == "aurora" ? 4 : 0;
        ps = new P[n];
        for (int i = 0; i < n; i++) {
            var p = new P { Ph = R(0, 6.28), K = i % 2 };
            switch (mode) {
                case "stars":  p.S = R(0.7, 1.8);  p.X = R(0, w); p.Y = R(0, h); p.VX = -R(0.1, 0.5); break;
                case "rain":   p.S = R(14, 44);    p.X = R(0, w); p.Y = R(-h, h); p.VY = R(6, 14); break;
                case "embers": p.S = R(1.2, 3.2);  p.X = R(0, w); p.Y = R(0, h); p.VY = -R(0.5, 1.7); break;
                case "aurora": p.S = R(280, 410); break;
            }
            ps[i] = p;
        }
        InvalidateVisual();
    }

    void Tick(object sender, EventArgs ev) {
        if (mode == "none" || ps.Length == 0) return;
        DateTime now = DateTime.UtcNow;
        if ((now - last).TotalMilliseconds < 33) return;
        last = now; t += 0.033;
        for (int i = 0; i < ps.Length; i++) {
            P p = ps[i];
            switch (mode) {
                case "stars": p.X += p.VX; if (p.X < -4) p.X = w + 4; break;
                case "rain":  p.Y += p.VY; if (p.Y > h) { p.Y = -p.S; p.X = R(0, w); } break;
                case "embers":
                    p.Y += p.VY; p.X += Math.Sin(t * 1.3 + p.Ph) * 0.5;
                    if (p.Y < -10) { p.Y = h + 10; p.X = R(0, w); }
                    break;
                case "aurora":
                    p.X = w * (0.15 + 0.35 * i / 3.0) + Math.Sin(t * 0.21 + p.Ph + i) * w * 0.28;
                    p.Y = h * (0.12 + 0.2 * i) + Math.Cos(t * 0.17 + p.Ph * 1.7) * h * 0.2;
                    break;
            }
        }
        InvalidateVisual();
    }

    int Level(double a) { return Math.Max(0, Math.Min(Levels - 1, (int)(a * Levels))); }

    protected override void OnRender(DrawingContext dc) {
        if (mode == "none") return;
        for (int i = 0; i < ps.Length; i++) {
            P p = ps[i];
            switch (mode) {
                case "stars":
                    dc.DrawEllipse((p.K == 0 ? ramp1 : ramp2)[Level(0.3 + 0.7 * Math.Abs(Math.Sin(t * 0.9 + p.Ph)))], null, new Point(p.X, p.Y), p.S, p.S);
                    break;
                case "rain":
                    dc.DrawRectangle((p.K == 0 ? ramp1 : ramp2)[3 + (int)(p.S % 4)], null, new Rect(p.X, p.Y, 1.4, p.S));
                    break;
                case "embers":
                    dc.DrawEllipse((p.K == 0 ? ramp1 : ramp2)[Level(Math.Max(0.1, Math.Min(0.9, p.Y / h)))], null, new Point(p.X, p.Y), p.S, p.S);
                    break;
                case "aurora":
                    dc.DrawEllipse(aurora[i], null, new Point(p.X, p.Y), p.S * 1.6, p.S);
                    break;
            }
        }
    }
}
'@
}
