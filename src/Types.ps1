# Compiled helpers: bindable list row + animated wallpaper effects.
if (-not ('OptiRow' -as [type])) {
    Add-Type -TypeDefinition @'
using System.ComponentModel;
public class OptiRow : INotifyPropertyChanged {
    public event PropertyChangedEventHandler PropertyChanged;
    void N(string p) { var h = PropertyChanged; if (h != null) h(this, new PropertyChangedEventArgs(p)); }
    string _name, _desc, _group, _risk, _extra; bool _chk;
    public string Id { get; set; }
    object _icon;
    public object Icon { get { return _icon; } set { _icon = value; N("Icon"); } }
    public string Initial { get { return string.IsNullOrEmpty(_name) ? "?" : _name.Substring(0, 1).ToUpper(); } }
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
